use std::{
	any::{Any, TypeId},
	fmt::{Debug, Display},
	future::Ready,
	sync::Arc,
};

use crate::{
	common::{mark::Private, patterns_to_route, BoxedFuture, IntoArray},
	handler::{
		request_handlers::{handle_misdirected_request, MethodHandlers},
		AdaptiveHandler, ArcHandler, HandlerKind, IntoArcHandler, IntoHandler,
	},
	middleware::{IntoResponseAdapter, LayerTarget, ResponseFutureBoxer},
	pattern::{Pattern, Similarity},
	request::Request,
	response::Response,
	routing::RouteSegments,
};

// --------------------------------------------------

mod futures;
mod service;
mod static_files;

use self::{
	futures::{RequestPasserFuture, RequestReceiverFuture},
	service::{request_handler, request_passer, request_receiver, InnerResource},
};

pub use service::ResourceService;
pub use static_files::{StaticFiles, Tagger};

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

pub struct Resource {
	pattern: Pattern,
	prefix_segment_patterns: Vec<Pattern>,

	static_resources: Vec<Resource>,
	regex_resources: Vec<Resource>,
	some_wildcard_resource: Option<Box<Resource>>,

	some_request_receiver: Option<ArcHandler>,
	some_request_passer: Option<ArcHandler>,
	some_request_handler: Option<ArcHandler>,

	method_handlers: MethodHandlers,
	some_misdirected_request_handler: Option<ArcHandler>,

	states: Vec<Box<dyn Any + Send + Sync>>,

	// TODO: configs, redirect
	is_subtree_handler: bool,
}

// -------------------------

impl Resource {
	pub fn new<P>(path_patterns: P) -> Resource
	where
		P: AsRef<str>,
	{
		let path_patterns = path_patterns.as_ref();

		if path_patterns.is_empty() {
			panic!("empty path patterns")
		}

		if path_patterns == "/" {
			let pattern = Pattern::parse(path_patterns);

			return Resource::with_pattern(pattern);
		}

		if !path_patterns.starts_with('/') {
			panic!("path patterns must start with a slash or must be a root pattern '/'")
		}

		let mut route_segments = RouteSegments::new(path_patterns);

		let mut prefix_path_patterns = Vec::new();

		let resource_pattern = loop {
			let (route_segment, _) = route_segments
				.next()
				.expect("local checks should validate that the next segment exists");

			let pattern = Pattern::parse(route_segment);

			if route_segments.has_remaining_segments() {
				prefix_path_patterns.push(pattern);

				continue;
			}

			break pattern;
		};

		Self::with_prefix_path_patterns(prefix_path_patterns, resource_pattern)
	}

	#[inline(always)]
	pub(crate) fn with_prefix_path_patterns(
		prefix_path_patterns: Vec<Pattern>,
		resource_pattern: Pattern,
	) -> Resource {
		if let Pattern::Regex(ref name, None) = resource_pattern {
			panic!("{} pattern has no regex segment", name.pattern_name())
		}

		Resource {
			pattern: resource_pattern,
			prefix_segment_patterns: prefix_path_patterns,
			static_resources: Vec::new(),
			regex_resources: Vec::new(),
			some_wildcard_resource: None,
			some_request_receiver: None,
			some_request_passer: None,
			some_request_handler: None,
			method_handlers: MethodHandlers::new(),
			some_misdirected_request_handler: None,
			states: Vec::new(),
			is_subtree_handler: false,
		}
	}

	#[inline(always)]
	pub(crate) fn with_pattern_str(pattern: &str) -> Resource {
		let pattern = Pattern::parse(pattern);

		Self::with_pattern(pattern)
	}

	#[inline(always)]
	pub(crate) fn with_pattern(pattern: Pattern) -> Resource {
		Self::with_prefix_path_patterns(Vec::new(), pattern)
	}

	// -------------------------

	#[inline(always)]
	fn name(&self) -> Option<&str> {
		self.pattern.name()
	}

	#[inline(always)]
	pub(crate) fn pattern(&self) -> String {
		self.pattern.to_string()
	}

	#[inline(always)]
	pub(crate) fn is_subtree_handler(&self) -> bool {
		self.is_subtree_handler
	}

	#[inline(always)]
	pub(crate) fn can_handle_request(&self) -> bool {
		!self.method_handlers.is_empty()
	}

	#[inline(always)]
	fn has_some_effect(&self) -> bool {
		self.method_handlers.has_some_effect()
			|| self.some_request_handler.is_some()
			|| self.some_request_passer.is_some()
			|| self.some_request_receiver.is_some()
	}

	// -------------------------

	pub fn add_subresource<R, const N: usize>(&mut self, new_resources: R)
	where
		R: IntoArray<Resource, N>,
	{
		let new_resources = new_resources.into_array();

		for new_resource in new_resources {
			self.add_single_subresource(new_resource);
		}
	}

	fn add_single_subresource(&mut self, mut new_resource: Resource) {
		if !new_resource.prefix_segment_patterns.is_empty() {
			let mut prefix_segment_patterns =
				std::mem::take(&mut new_resource.prefix_segment_patterns).into_iter();

			self.check_path_segments_are_the_same(&mut prefix_segment_patterns);

			if prefix_segment_patterns.len() > 0 {
				// We must create the remaining prefix segment resources and get the last subresource
				// to be a parent of the new resource.
				let subresource_to_be_parent = self.by_patterns_subresource_mut(prefix_segment_patterns);
				subresource_to_be_parent.add_subresource(new_resource);

				return;
			}
		};

		self.check_names_are_unique_in_the_path(&new_resource);

		// -----

		macro_rules! add_resource {
			($resources:expr, $new_resource:ident) => {
				// Resources that do not affect requests may exist in the resource tree. They haven't
				// been wrapped in any middleware and don't have any request handler. These resources
				// may be replaced by another resource with the same pattern if that resource has some
				// effect. Here we go through a new resource and its subresources, compare them to the
				// resources in the existing resource tree and keep one or the other. If two matching
				// resources both have an effect, that's a bug on the application's side.

				// We don't want to lock $resources by getting a mutable reference to a matching
				// resource. So we'll find its position instead.
				if let Some(position) = $resources
					.iter_mut()
					.position(|resource| resource.pattern.compare(&$new_resource.pattern) == Similarity::Same)
				{
					// We found its position. Now we must own it :)
					let dummy_resource = Resource::with_pattern(Pattern::default());
					let mut existing_resource = std::mem::replace(&mut $resources[position], dummy_resource);

					if !$new_resource.has_some_effect() {
						existing_resource.keep_subresources($new_resource);
					} else if !existing_resource.has_some_effect() {
						// We can't just replace the existing resource with a new resource. The new resource
						// must also keep the prefix segment patterns of the existing resource.
						$new_resource.prefix_segment_patterns =
							std::mem::take(&mut existing_resource.prefix_segment_patterns);

						$new_resource.keep_subresources(existing_resource);
						existing_resource = $new_resource;
					} else {
						// Both matching resources are valid resources with some effect.
						panic!(
							"conflicting resources with a pattern '{}'",
							$new_resource.pattern
						)
					}

					$resources[position] = existing_resource;
				} else {
					$new_resource.prefix_segment_patterns = self.path_patterns();
					$resources.push($new_resource);
				}
			};
		}

		// -----

		match &new_resource.pattern {
			Pattern::Static(_) => add_resource!(self.static_resources, new_resource),
			Pattern::Regex(..) => add_resource!(self.regex_resources, new_resource),
			Pattern::Wildcard(_) => {
				// Explanation inside the above macro 'add_resource!' also applies here.
				if let Some(mut wildcard_resource) = self.some_wildcard_resource.take() {
					if wildcard_resource.pattern.compare(&new_resource.pattern) == Similarity::Same {
						if !new_resource.has_some_effect() {
							wildcard_resource.keep_subresources(new_resource);
						} else if !wildcard_resource.has_some_effect() {
							new_resource.prefix_segment_patterns =
								std::mem::take(&mut wildcard_resource.prefix_segment_patterns);

							new_resource.keep_subresources(*wildcard_resource);
							*wildcard_resource = new_resource;
						} else {
							panic!(
								"conflicting resources with a pattern '{}'",
								new_resource.pattern
							)
						}
					} else {
						panic!("resource can have only one child resource with a wildcard pattern")
					}

					self.some_wildcard_resource = Some(wildcard_resource);
				} else {
					new_resource.prefix_segment_patterns = self.path_patterns();
					self.some_wildcard_resource = Some(Box::new(new_resource));
				}
			}
		}
	}

	#[inline]
	fn check_path_segments_are_the_same(
		&self,
		prefix_segment_patterns: &mut impl Iterator<Item = Pattern>,
	) {
		let self_path_segment_patterns = self
			.prefix_segment_patterns
			.iter()
			.chain(std::iter::once(&self.pattern));

		for self_path_segment_pattern in self_path_segment_patterns {
			let Some(prefix_segment_pattern) = prefix_segment_patterns.next() else {
				panic!("prefix path patterns must be the same with the path patterns of the parent")
			};

			// For convenience, resource's prefix segment patterns may omit their regex part.
			// So when matching them to the parent resource's path segment patterns, we only
			// compare pattern names.
			if let Pattern::Regex(prefix_segment_names, None) = &prefix_segment_pattern {
				if let Pattern::Regex(self_path_segment_names, _) = self_path_segment_pattern {
					if prefix_segment_names.pattern_name() == self_path_segment_names.pattern_name() {
						continue;
					}
				}

				panic!(
					"no prefix path segment resource with a name '{}' exists",
					prefix_segment_names.pattern_name(),
				)
			}

			if self_path_segment_pattern.compare(&prefix_segment_pattern) != Similarity::Same {
				panic!(
					"no segment '{}' exists among the prefix path segments of the resource '{}",
					prefix_segment_pattern,
					self.pattern(),
				)
			}
		}
	}

	#[inline]
	fn by_patterns_subresource_mut(
		&mut self,
		patterns: impl Iterator<Item = Pattern>,
	) -> &mut Resource {
		let (leaf_resource_in_the_path, patterns) = self.by_patterns_leaf_resource_mut(patterns);
		leaf_resource_in_the_path.by_patterns_new_subresource_mut(patterns)
	}

	// Iterates over the patterns matching them to self and the corresponding subresources.
	// Returns the last matching subresource and the remaining patterns.
	fn by_patterns_leaf_resource_mut(
		&mut self,
		patterns: impl Iterator<Item = Pattern>,
	) -> (&mut Resource, impl Iterator<Item = Pattern>) {
		let mut leaf_resource = self;

		let mut peekable_patterns = patterns.peekable();
		while let Some(pattern) = peekable_patterns.peek() {
			match pattern {
				Pattern::Static(_) => {
					let some_position = leaf_resource
						.static_resources
						.iter()
						.position(|resource| resource.pattern.compare(pattern) == Similarity::Same);

					if let Some(position) = some_position {
						leaf_resource = &mut leaf_resource.static_resources[position];
						peekable_patterns.next();
					} else {
						break;
					}
				}
				Pattern::Regex(ref name, ref some_regex) => {
					let some_position = leaf_resource.regex_resources.iter().position(|resource| {
						if some_regex.is_some() {
							resource.pattern.compare(pattern) == Similarity::Same
						} else {
							resource.name().expect("regex resources must have a name") == name.pattern_name()
						}
					});

					if let Some(position) = some_position {
						leaf_resource = &mut leaf_resource.regex_resources[position];
						peekable_patterns.next();
					} else {
						break;
					}
				}
				Pattern::Wildcard(_) => {
					if leaf_resource
						.some_wildcard_resource
						.as_ref()
						.is_some_and(|resource| resource.pattern.compare(pattern) == Similarity::Same)
					{
						leaf_resource = leaf_resource
							.some_wildcard_resource
							.as_deref_mut()
							.expect("if statement should prove that the wildcard resource exists");

						peekable_patterns.next();
					} else {
						break;
					}
				}
			}
		}

		(leaf_resource, peekable_patterns)
	}

	#[inline]
	fn by_patterns_new_subresource_mut(
		&mut self,
		patterns: impl Iterator<Item = Pattern>,
	) -> &mut Resource {
		let mut current_resource = self;

		for pattern in patterns {
			if let Some(name) = pattern.name() {
				if current_resource.path_has_the_same_name(name) {
					panic!("{} is not unique in the path", name)
				}
			}

			let pattern_clone = pattern.clone();
			let new_subresource = Resource::with_pattern(pattern);
			current_resource.add_subresource(new_subresource);

			(current_resource, _) =
				current_resource.by_patterns_leaf_resource_mut(std::iter::once(pattern_clone));
		}

		current_resource
	}

	// Checks the names of the new resource and its subresources for uniqueness.
	#[inline]
	fn check_names_are_unique_in_the_path(&self, new_resource: &Resource) {
		if let Some(name) = new_resource.name() {
			if self.path_has_the_same_name(name) {
				panic!("'{}' is not unique in the path it's being added", name);
			}
		}

		let mut resources = Vec::new();
		resources.extend(new_resource.regex_resources.iter());

		if let Some(wildcard_resource) = &new_resource.some_wildcard_resource {
			resources.push(wildcard_resource);
		}

		loop {
			let Some(resource) = resources.pop() else {
				return;
			};

			let name = resource
				.name()
				.expect("regex and wildcard resources must have a name");
			if self.path_has_the_same_name(name) {
				panic!("'{}' is not unique in the path it's being added", name);
			}

			resources.extend(resource.regex_resources.iter());

			if let Some(wildcard_resource) = &resource.some_wildcard_resource {
				resources.push(wildcard_resource);
			}
		}
	}

	// Tries to compare all the subresources of the other with the corresponding subresources
	// of self and keeps the ones that have some effect on a request or have no corresponding
	// resource.
	fn keep_subresources(&mut self, mut other: Resource) {
		macro_rules! keep_other_resources {
			(mut $resources:expr, mut $other_resources:expr) => {
				if !$other_resources.is_empty() {
					if $resources.is_empty() {
						for other_resource in $other_resources.iter_mut() {
							other_resource.prefix_segment_patterns = self.path_patterns();
						}

						$resources = $other_resources;
					} else {
						for mut other_resource in $other_resources {
							if let Some(position) = $resources.iter().position(|resource| {
								resource.pattern.compare(&other_resource.pattern) == Similarity::Same
							}) {
								let dummy_resource = Resource::with_pattern(Pattern::default());
								let mut resource = std::mem::replace(&mut $resources[position], dummy_resource);

								if !other_resource.has_some_effect() {
									resource.keep_subresources(other_resource);
								} else if !resource.has_some_effect() {
									other_resource.prefix_segment_patterns =
										std::mem::take(&mut resource.prefix_segment_patterns);

									other_resource.keep_subresources(resource);
									resource = other_resource;
								} else {
									panic!(
										"conflicting resources with a pattern '{}'",
										other_resource.pattern
									)
								}

								$resources[position] = resource;
							} else {
								other_resource.prefix_segment_patterns = self.path_patterns();
								$resources.push(other_resource);
							}
						}
					}
				}
			};
		}

		// -----

		keep_other_resources!(mut self.static_resources, mut other.static_resources);

		keep_other_resources!(mut self.regex_resources, mut other.regex_resources);

		if let Some(mut other_wildcard_resource) = other.some_wildcard_resource.take() {
			if let Some(mut wildcard_resource) = self.some_wildcard_resource.take() {
				if wildcard_resource
					.pattern
					.compare(&other_wildcard_resource.pattern)
					== Similarity::Same
				{
					if !other_wildcard_resource.has_some_effect() {
						wildcard_resource.keep_subresources(*other_wildcard_resource);
					} else if !wildcard_resource.has_some_effect() {
						other_wildcard_resource.prefix_segment_patterns =
							std::mem::take(&mut wildcard_resource.prefix_segment_patterns);
						other_wildcard_resource.keep_subresources(*wildcard_resource);
						wildcard_resource = other_wildcard_resource;
					} else {
						// TODO: Improve the error message.
						panic!("sub resource has duplicate wildcard pattern")
					}
				} else {
					// TODO: Improve the error message.
					panic!("sub resource has wildcard pattern with different name")
				}

				self.some_wildcard_resource = Some(wildcard_resource);
			} else {
				other_wildcard_resource.prefix_segment_patterns = self.path_patterns();
				self.some_wildcard_resource = Some(other_wildcard_resource);
			}
		}
	}

	#[inline(always)]
	fn path_patterns(&self) -> Vec<Pattern> {
		let mut prefix_patterns = self.prefix_segment_patterns.clone();
		prefix_patterns.push(self.pattern.clone());

		prefix_patterns
	}

	pub fn add_subresource_under<P, R, const N: usize>(&mut self, relative_path: P, new_resources: R)
	where
		P: AsRef<str>,
		R: IntoArray<Resource, N>,
	{
		let relative_path = relative_path.as_ref();
		let new_resources = new_resources.into_array();

		for new_resource in new_resources {
			self.add_single_subresource_under(relative_path, new_resource);
		}
	}

	fn add_single_subresource_under(&mut self, relative_path: &str, mut new_resource: Resource) {
		if !new_resource.prefix_segment_patterns.is_empty() {
			let mut prefix_segment_patterns =
				std::mem::take(&mut new_resource.prefix_segment_patterns).into_iter();

			// Prefix segments start from the root. They must be the same as the path segments of self.
			self.check_path_segments_are_the_same(&mut prefix_segment_patterns);

			if relative_path.is_empty() {
				if prefix_segment_patterns.len() > 0 {
					// There are remaining segments that we need to create corresponding subresources.
					let subresource_to_be_parent = self.by_patterns_subresource_mut(prefix_segment_patterns);

					subresource_to_be_parent.add_subresource(new_resource);
				} else {
					self.add_subresource(new_resource);
				}

				return;
			}

			// Keeps the prefix route patterns
			let mut prefix_route_patterns = Vec::new();

			let prefix_route_segments = RouteSegments::new(relative_path);
			for (prefix_route_segment, _) in prefix_route_segments {
				let Some(prefix_segment_pattern) = prefix_segment_patterns.next() else {
					panic!(
						"new resource has fewer prefix path segments specified than where it's being added",
					)
				};

				// Keeps the complete prefix segment patterns to construct subresources later.
				let prefix_route_segment_pattern = Pattern::parse(prefix_route_segment);

				// For convenience, regex part of the pattern may be omitted.
				// So when matching them we can compare only the names.
				if let Pattern::Regex(ref prefix_route_segment_names, None) = prefix_route_segment_pattern {
					if let Pattern::Regex(prefix_segment_names, some_regex) = &prefix_segment_pattern {
						if some_regex.is_none() {
							panic!("either route's segment or the resource's prefix segment must be complete")
						}

						if prefix_segment_names.pattern_name() == prefix_route_segment_names.pattern_name() {
							prefix_route_patterns.push(prefix_segment_pattern);

							continue;
						}
					}
				} else if let Pattern::Regex(ref prefix_segment_names, None) = prefix_segment_pattern {
					if let Pattern::Regex(prefix_route_segment_names, some_regex) =
						&prefix_route_segment_pattern
					{
						if some_regex.is_none() {
							panic!("either route's segment or the resource's prefix segment must be complete")
						}

						if prefix_route_segment_names.pattern_name() == prefix_segment_names.pattern_name() {
							prefix_route_patterns.push(prefix_route_segment_pattern);

							continue;
						}
					}
				}

				if prefix_route_segment_pattern.compare(&prefix_segment_pattern) != Similarity::Same {
					panic!(
						"resource's prefix segment pattern didn't match to the route's corresponding segment",
					)
				}

				prefix_route_patterns.push(prefix_route_segment_pattern);
			}

			let mut subresource_to_be_parent =
				self.by_patterns_subresource_mut(prefix_route_patterns.into_iter());

			if prefix_segment_patterns.len() > 0 {
				// We were given fewer segments in the route and the resource still has some
				// remaining prefix segments that need corresponding resources to be created.
				subresource_to_be_parent =
					subresource_to_be_parent.by_patterns_subresource_mut(prefix_segment_patterns);
			}

			subresource_to_be_parent.add_subresource(new_resource);

			return;
		}

		if relative_path.is_empty() {
			self.add_subresource(new_resource);
		} else {
			let subresource_to_be_parent = self.subresource_mut(relative_path);
			subresource_to_be_parent.add_subresource(new_resource);
		}
	}

	pub fn subresource_mut<P>(&mut self, relative_path: P) -> &mut Resource
	where
		P: AsRef<str>,
	{
		let relative_path = relative_path.as_ref();

		if relative_path.is_empty() {
			panic!("empty route")
		}

		if relative_path == "/" {
			panic!("root cannot be a sub-resource")
		}

		if !relative_path.starts_with('/') {
			panic!("{} route must start with '/'", relative_path)
		}

		let segments = RouteSegments::new(relative_path);
		let (leaf_resource_in_the_path, segments) = self.leaf_resource_mut(segments);

		leaf_resource_in_the_path.new_subresource_mut(segments)
	}

	fn leaf_resource<'s, 'r>(
		&'s self,
		mut route_segments: RouteSegments<'r>,
	) -> (&'s Resource, RouteSegments<'r>) {
		let mut leaf_resource = self;

		for (segment, segment_index) in route_segments.by_ref() {
			let pattern = Pattern::parse(segment);

			match pattern {
				Pattern::Static(_) => {
					let some_position = leaf_resource
						.static_resources
						.iter()
						.position(|resource| resource.pattern.compare(&pattern) == Similarity::Same);

					if let Some(position) = some_position {
						leaf_resource = &leaf_resource.static_resources[position];
					} else {
						route_segments.revert_to_segment(segment_index);

						break;
					}
				}
				Pattern::Regex(ref names, ref some_regex) => {
					let some_position = leaf_resource.regex_resources.iter().position(|resource| {
						if some_regex.is_some() {
							resource.pattern.compare(&pattern) == Similarity::Same
						} else {
							resource.name().expect("regex resources must have a name") == names.pattern_name()
						}
					});

					if let Some(position) = some_position {
						leaf_resource = &leaf_resource.regex_resources[position];
					} else {
						route_segments.revert_to_segment(segment_index);

						break;
					}
				}
				Pattern::Wildcard(_) => {
					if leaf_resource
						.some_wildcard_resource
						.as_ref()
						.is_some_and(|resource| resource.pattern.compare(&pattern) == Similarity::Same)
					{
						leaf_resource = leaf_resource
							.some_wildcard_resource
							.as_deref()
							.expect("if statement should prove that the wildcard resource exists");
					} else {
						route_segments.revert_to_segment(segment_index);

						break;
					}
				}
			}
		}

		(leaf_resource, route_segments)
	}

	fn leaf_resource_mut<'s, 'r>(
		&'s mut self,
		mut route_segments: RouteSegments<'r>,
	) -> (&'s mut Resource, RouteSegments<'r>) {
		let mut leaf_resource = self;

		for (segment, segment_index) in route_segments.by_ref() {
			let pattern = Pattern::parse(segment);

			match pattern {
				Pattern::Static(_) => {
					let some_position = leaf_resource
						.static_resources
						.iter()
						.position(|resource| resource.pattern.compare(&pattern) == Similarity::Same);

					if let Some(position) = some_position {
						leaf_resource = &mut leaf_resource.static_resources[position];
					} else {
						route_segments.revert_to_segment(segment_index);

						break;
					}
				}
				Pattern::Regex(ref names, ref some_regex) => {
					let some_position = leaf_resource.regex_resources.iter().position(|resource| {
						if some_regex.is_some() {
							resource.pattern.compare(&pattern) == Similarity::Same
						} else {
							resource.name().expect("regex resources must have a name") == names.pattern_name()
						}
					});

					if let Some(position) = some_position {
						leaf_resource = &mut leaf_resource.regex_resources[position];
					} else {
						route_segments.revert_to_segment(segment_index);

						break;
					}
				}
				Pattern::Wildcard(_) => {
					if leaf_resource
						.some_wildcard_resource
						.as_ref()
						.is_some_and(|resource| resource.pattern.compare(&pattern) == Similarity::Same)
					{
						leaf_resource = leaf_resource
							.some_wildcard_resource
							.as_deref_mut()
							.expect("if statement should prove that the wildcard resource exists");
					} else {
						route_segments.revert_to_segment(segment_index);

						break;
					}
				}
			}
		}

		(leaf_resource, route_segments)
	}

	#[inline]
	fn new_subresource_mut(&mut self, segments: RouteSegments) -> &mut Resource {
		let mut current_resource = self;

		for (segment, _) in segments {
			let pattern = Pattern::parse(segment);

			if let Some(name) = pattern.name() {
				if current_resource.path_has_the_same_name(name) {
					panic!("{} is not unique in the path", name)
				}
			}

			let new_subresource = Resource::with_pattern(pattern);
			current_resource.add_subresource(new_subresource);
			(current_resource, _) = current_resource.leaf_resource_mut(RouteSegments::new(segment));
		}

		current_resource
	}

	#[inline]
	fn path_has_the_same_name(&self, name: &str) -> bool {
		if let Some(resource_name) = self.name() {
			if resource_name == name {
				return true;
			}
		}

		for prefix_pattern in self.prefix_segment_patterns.iter() {
			if let Some(pattern_name) = prefix_pattern.name() {
				if pattern_name == name {
					return true;
				}
			}
		}

		false
	}

	// -------------------------

	pub fn add_state<S: Clone + Send + Sync + 'static>(&mut self, state: S) {
		let state_type_id = state.type_id();

		if self
			.states
			.iter()
			.any(|existing_state| (*(*existing_state)).type_id() == state_type_id)
		{
			panic!(
				"resource already has a state of type '{:?}'",
				TypeId::of::<S>()
			);
		}

		self.states.push(Box::new(state));
	}

	pub fn state_ref<S: Clone + Send + Sync + 'static>(&self) -> &S {
		self
			.states
			.iter()
			.find_map(|state| state.downcast_ref::<S>())
			.expect(&format!(
				"resource has no state of type '{:?}'",
				TypeId::of::<S>()
			))
	}

	pub fn set_handler_for<H, const N: usize>(&mut self, handler_kinds: H)
	where
		H: IntoArray<HandlerKind, N>,
	{
		let handler_kinds = handler_kinds.into_array();
		for handler_kind in handler_kinds {
			use crate::handler::Inner::*;

			match handler_kind.0 {
				Method(method, handler) => {
					let ready_handler =
						ResponseFutureBoxer::wrap(IntoResponseAdapter::wrap(handler.into_handler()));

					self
						.method_handlers
						.set_for(method, ready_handler.into_arc_handler())
				}
				AllMethods(handler) => {
					let ready_handler =
						ResponseFutureBoxer::wrap(IntoResponseAdapter::wrap(handler.into_handler()));

					self
						.method_handlers
						.set_for_all_methods(ready_handler.into_arc_handler())
				}
				MisdirectedRequest(handler) => {
					let ready_handler =
						ResponseFutureBoxer::wrap(IntoResponseAdapter::wrap(handler.into_handler()));

					self.some_misdirected_request_handler = Some(ready_handler.into_arc_handler())
				}
			}
		}
	}

	pub fn wrap<L, const N: usize>(&mut self, layer_targets: L)
	where
		L: IntoArray<LayerTarget, N>,
	{
		let layer_targets = layer_targets.into_array();
		for layer_target in layer_targets {
			use crate::middleware::Inner::*;

			match layer_target.0 {
				RequestReceiver(boxed_layer) => {
					let arc_request_receiver = match self.some_request_receiver.take() {
						Some(request_receiver) => request_receiver,
						None => {
							let request_receiver = <fn(Request) -> RequestReceiverFuture as IntoHandler<(
								Private,
								Request,
							)>>::into_handler(request_receiver);

							ResponseFutureBoxer::wrap(request_receiver).into_arc_handler()
						}
					};

					let arc_request_receiver = boxed_layer.wrap(AdaptiveHandler::from(arc_request_receiver));
					self.some_request_receiver.replace(arc_request_receiver);
				}
				RequestPasser(boxed_layer) => {
					let arc_request_passer = match self.some_request_passer.take() {
						Some(request_passer) => request_passer,
						None => {
							let request_passer = <fn(Request) -> RequestPasserFuture as IntoHandler<(
								Private,
								Request,
							)>>::into_handler(request_passer);

							ResponseFutureBoxer::wrap(request_passer).into_arc_handler()
						}
					};

					let arc_request_passer = boxed_layer.wrap(AdaptiveHandler::from(arc_request_passer));
					self.some_request_passer.replace(arc_request_passer);
				}
				RequestHandler(boxed_layer) => {
					let arc_request_handler = match self.some_request_handler.take() {
						Some(request_handler) => request_handler,
						None => {
							let request_handler = <fn(Request) -> BoxedFuture<Response> as IntoHandler<
								Request,
							>>::into_handler(request_handler);

							request_handler.into_arc_handler()
						}
					};

					let arc_request_handler = boxed_layer.wrap(AdaptiveHandler::from(arc_request_handler));
					self.some_request_handler.replace(arc_request_handler);
				}
				MethodHandler(methods, arc_layer) => {
					for method in methods {
						self
							.method_handlers
							.wrap_handler_of(method, arc_layer.clone());
					}
				}
				AllMethodsHandler(boxed_layer) => {
					self.method_handlers.wrap_all_methods_handler(boxed_layer);
				}
				MisdirectedRequestHandler(boxed_layer) => {
					let arc_misdirected_request_handler = match self.some_misdirected_request_handler.take() {
						Some(misdirected_request_handler) => misdirected_request_handler,
						None => {
							let misdirected_request_handler = <fn(Request) -> Ready<Response> as IntoHandler<
								(Private, Request),
							>>::into_handler(handle_misdirected_request);

							ResponseFutureBoxer::wrap(misdirected_request_handler).into_arc_handler()
						}
					};

					let arc_misdirected_request_handler =
						boxed_layer.wrap(AdaptiveHandler::from(arc_misdirected_request_handler));

					self
						.some_misdirected_request_handler
						.replace(arc_misdirected_request_handler);
				}
			}
		}
	}

	// -------------------------

	pub fn for_each_subresource<T, F>(&mut self, mut param: T, mut func: F) -> T
	where
		F: FnMut(&mut T, &mut Resource) -> Iteration,
	{
		let mut subresources = Vec::new();
		subresources.extend(self.static_resources.iter_mut());
		subresources.extend(self.regex_resources.iter_mut());
		if let Some(resource) = self.some_wildcard_resource.as_deref_mut() {
			subresources.push(resource);
		}

		loop {
			let Some(subresource) = subresources.pop() else {
				break param;
			};

			match func(&mut param, subresource) {
				Iteration::SkipSubtree => continue,
				Iteration::Stop => break param,
				_ => {}
			}

			subresources.extend(subresource.static_resources.iter_mut());
			subresources.extend(subresource.regex_resources.iter_mut());
			if let Some(resource) = subresource.some_wildcard_resource.as_deref_mut() {
				subresources.push(resource);
			}
		}
	}

	pub fn into_service(self) -> ResourceService {
		let Resource {
			pattern,
			static_resources,
			regex_resources,
			some_wildcard_resource: wildcard_resource,
			some_request_receiver: request_receiver,
			some_request_passer: request_passer,
			some_request_handler: request_handler,
			method_handlers,
			states: state,
			is_subtree_handler,
			..
		} = self;

		let static_resources = if static_resources.is_empty() {
			None
		} else {
			Some(
				static_resources
					.into_iter()
					.map(Resource::into_service)
					.collect(),
			)
		};

		let regex_resources = if regex_resources.is_empty() {
			None
		} else {
			Some(
				regex_resources
					.into_iter()
					.map(Resource::into_service)
					.collect(),
			)
		};

		let wildcard_resource = wildcard_resource.map(|resource| resource.into_service());

		ResourceService(Arc::new(InnerResource {
			pattern,
			static_resources,
			regex_resources,
			wildcard_resource,
			request_receiver,
			request_passer,
			request_handler,
			method_handlers,
			state: if state.is_empty() {
				None
			} else {
				Some(Arc::from(state))
			},
			is_subtree_handler,
		}))
	}
}

impl Display for Resource {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(
			f,
			"{}/{}",
			patterns_to_route(&self.prefix_segment_patterns),
			&self.pattern
		)
	}
}

impl Debug for Resource {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(
			f,
			"Resource {{
				pattern: {},
				prefix_segment_patterns: {},
				static_resources count: {},
				regex_resources count: {},
				wildcard_resource exists: {},
				layered request_receiver exists: {},
				layered request_passer exists: {},
				layered request_handler exists: {},
				method_handlers: {{ count: {}, unsupported_method_handler_exists: {} }},
				misdirected_request_handler exists: {},
				states count: {},
				is_subtree_handler: {},
			}}",
			&self.pattern,
			patterns_to_route(&self.prefix_segment_patterns),
			self.static_resources.len(),
			self.regex_resources.len(),
			self.some_wildcard_resource.is_some(),
			self.some_request_receiver.is_some(),
			self.some_request_passer.is_some(),
			self.some_request_handler.is_some(),
			self.method_handlers.count(),
			self.method_handlers.has_all_methods_handler(),
			self.some_misdirected_request_handler.is_some(),
			self.states.len(),
			self.is_subtree_handler,
		)
	}
}

impl IntoArray<Resource, 1> for Resource {
	fn into_array(self) -> [Resource; 1] {
		[self]
	}
}

// -------------------------

#[repr(u8)]
pub enum Iteration {
	Continue,
	SkipSubtree,
	Stop,
}

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

#[cfg(test)]
mod test {
	use crate::{
		common::route_to_patterns,
		handler::{futures::DefaultResponseFuture, get, post, put, DummyHandler},
	};

	use super::*;

	// --------------------------------------------------------------------------------

	#[test]
	fn new() {
		let path_patterns = [
			"/news",
			"/news/$area:@(local|worldwide)",
			"/products/",
			"/products/*category",
			"/products/*category/$page:@(\\d+)/",
			"/$forecast:@days(5|10)-forecast",
			"/*random",
		];

		for path_pattern in path_patterns {
			let result = Resource::new(path_pattern);
			println!("path pattern: {}\n\t resource: {}", path_pattern, result);
		}
	}

	#[test]
	#[should_panic(expected = "empty path patterns")]
	fn new_with_empty_pattern() {
		Resource::new("");
	}

	#[test]
	#[should_panic(expected = "must start with a slash")]
	fn new_with_invalid_path_patterns() {
		Resource::new("products/*category");
	}

	#[test]
	fn add_subresource() {
		let mut parent = Resource::new("/abc0_0/*abc1_0");

		let cases = [
			("/abc0_0/*abc1_0/$abc2_0:@(p0)/", "/$abc2_0:@(p0)/"),
			(
				"/abc0_0/*abc1_0/$abc2_0:@(p0)/abc3_0",
				"/$abc2_0:@(p0)/abc3_0",
			),
			(
				"/abc0_0/*abc1_0/$abc2_0:@(p0)/$abc3_1:@cn0(p0)",
				"/$abc2_0:@(p0)/$abc3_1:@cn0(p0)",
			),
			(
				"/abc0_0/*abc1_0/$abc2_0:@(p0)/$abc3_1:@cn0(p0)/*abc4_0",
				"/$abc2_0:@(p0)/$abc3_1:@cn0(p0)/*abc4_0",
			),
			(
				"/abc0_0/*abc1_0/$abc2_0:@(p0)/$abc3_1:@cn0(p0)/abc4_1",
				"/$abc2_0:@(p0)/$abc3_1:@cn0(p0)/abc4_1",
			),
			(
				"/abc0_0/*abc1_0/$abc2_0:@(p0)/$abc3_1:@cn0(p0)/abc4_2",
				"/$abc2_0:@(p0)/$abc3_1:@cn0(p0)/abc4_2",
			),
			("/abc0_0/*abc1_0/abc2_1", "/abc2_1"),
			(
				"/abc0_0/*abc1_0/abc2_1/*abc3_0/$abc4_0:@(p0)/",
				"/abc2_1/*abc3_0/$abc4_0:@(p0)",
			),
			(
				"/abc0_0/*abc1_0/abc2_1/*abc3_0/$abc4_1:@(p1)",
				"/abc2_1/*abc3_0/$abc4_1:@(p1)/",
			),
			("/abc0_0/*abc1_0/abc2_1/*abc3_0", "/abc2_1/*abc3_0"),
		];

		for case in cases {
			let resource = Resource::new(case.0);

			parent.add_subresource(resource);

			let (resource, _) =
				parent.by_patterns_leaf_resource_mut(route_to_patterns(case.1).into_iter());
			resource.check_path_segments_are_the_same(&mut route_to_patterns(case.0).into_iter());
		}

		{
			// Existing resources in the tree.

			let (resource2_0, _) = parent.leaf_resource_mut(RouteSegments::new("/$abc2_0:@(p0)"));
			resource2_0.set_handler_for(post(DummyHandler::<DefaultResponseFuture>::new()));
			resource2_0
				.subresource_mut("/$abc3_1:@cn0(p0)/*abc4_0")
				.set_handler_for(get(DummyHandler::<DefaultResponseFuture>::new()));

			let (resource4_2, _) =
				resource2_0.leaf_resource_mut(RouteSegments::new("/$abc3_1:@cn0(p0)/abc4_2"));
			resource4_2.new_subresource_mut(RouteSegments::new("/abc5_0"));
		}

		{
			// New resources.

			let mut resource2_0 = Resource::new("/$abc2_0:@(p0)");

			let mut resource3_1 = Resource::new("/$abc3_1:@cn0(p0)");
			resource3_1.set_handler_for([
				get(DummyHandler::<DefaultResponseFuture>::new()),
				post(DummyHandler::<DefaultResponseFuture>::new()),
			]);

			resource3_1
				.subresource_mut("/abc4_1")
				.set_handler_for(post(DummyHandler::<DefaultResponseFuture>::new()));
			resource3_1.new_subresource_mut(RouteSegments::new("/$abc4_3:@(p0)"));
			resource3_1.new_subresource_mut(RouteSegments::new("/abc4_4"));

			resource2_0.add_subresource(resource3_1);

			// Resources with handlers must replace existing resources with the same pattern.
			// Other resources must be kept as is. New subtree must be a union of the existing two subtrees.
			parent.add_subresource(resource2_0);

			let (resource2_0, _) = parent.leaf_resource(RouteSegments::new("/$abc2_0:@(p0)"));
			assert_eq!(resource2_0.static_resources.len(), 1);
			assert_eq!(resource2_0.regex_resources.len(), 1);
			assert_eq!(resource2_0.method_handlers.count(), 1);

			let (resource3_1, _) =
				parent.leaf_resource(RouteSegments::new("/$abc2_0:@(p0)/$abc3_1:@cn0(p0)"));
			assert_eq!(resource3_1.static_resources.len(), 3);
			assert_eq!(resource3_1.regex_resources.len(), 1);
			assert!(resource3_1.some_wildcard_resource.is_some());
			assert_eq!(resource3_1.method_handlers.count(), 2);

			let (resource4_0, _) = resource3_1.leaf_resource(RouteSegments::new("/*abc4_0"));
			assert_eq!(resource4_0.method_handlers.count(), 1);

			let (resource4_2, _) = resource3_1.leaf_resource(RouteSegments::new("/abc4_2"));
			assert_eq!(resource4_2.static_resources.len(), 1);

			let (resource5_0, _) = resource4_2.leaf_resource(RouteSegments::new("/abc5_0"));
			resource5_0.check_path_segments_are_the_same(
				&mut route_to_patterns("/abc0_0/*abc1_0/$abc2_0:@(p0)/$abc3_1:@cn0(p0)/abc4_2/abc5_0")
					.into_iter(),
			);

			let (resource4_1, _) = resource3_1.leaf_resource(RouteSegments::new("/abc4_1"));
			assert_eq!(resource4_1.method_handlers.count(), 1);
		}

		{
			let pattern3_0 = "/abc0_0/*abc1_0/abc2_1/*abc3_0";
			let route3_0 = "/abc2_1/*abc3_0";

			let mut resource3_0 = Resource::new(pattern3_0);
			resource3_0.by_patterns_new_subresource_mut(std::iter::once(Pattern::parse("abc4_2")));
			resource3_0.set_handler_for(get(DummyHandler::<DefaultResponseFuture>::new()));

			parent.add_subresource(resource3_0);
			let (resource3_0, _) = parent.leaf_resource_mut(RouteSegments::new(route3_0));
			resource3_0.check_path_segments_are_the_same(&mut route_to_patterns(pattern3_0).into_iter());
			assert_eq!(resource3_0.static_resources.len(), 1);
			assert_eq!(resource3_0.regex_resources.len(), 2);
			assert_eq!(resource3_0.method_handlers.count(), 1);
		}
	}

	#[test]
	fn check_path_segments_are_the_same() {
		let path_patterns = [
			("/news", "/news"),
			(
				"/news/$area:@(local|worldwide)",
				"/news/$area:@(local|worldwide)",
			),
			("/products/", "/products/"),
			("/products/*category", "/products/*category"),
			(
				"/products/*category/$page:@(\\d+)/",
				"/products/*category/$page/",
			),
			("/$forecast:@days(5|10)-forecast/*city", "/$forecast/*city"),
		];

		for segment_patterns in path_patterns {
			let resource = Resource::new(segment_patterns.0);
			let segmets = RouteSegments::new(segment_patterns.1);
			let mut segment_patterns = Vec::new();
			for (segment, _) in segmets {
				let pattern = Pattern::parse(segment);
				segment_patterns.push(pattern);
			}

			resource.check_path_segments_are_the_same(&mut segment_patterns.into_iter());
		}
	}

	#[test]
	#[should_panic]
	fn check_path_segments_are_the_same_panic1() {
		let resource = Resource::new("/news/$area:@(local|worldwide)");
		let mut segment_patterns = vec![Pattern::parse("news"), Pattern::parse("local")].into_iter();

		resource.check_path_segments_are_the_same(&mut segment_patterns);
	}

	#[test]
	#[should_panic(expected = "no prefix path segment resource")]
	fn check_path_segments_are_the_same_panic2() {
		let resource = Resource::new("/news/$area:@(local|worldwide)");
		let mut segment_patterns = vec![Pattern::parse("news"), Pattern::parse("$city")].into_iter();

		resource.check_path_segments_are_the_same(&mut segment_patterns);
	}

	#[test]
	#[should_panic(expected = "no segment '*area' exists among the prefix path segments")]
	fn check_path_segments_are_the_same_panic3() {
		let resource = Resource::new("/news/$area:@(local|worldwide)");
		let mut segment_patterns = vec![Pattern::parse("news"), Pattern::parse("*area")].into_iter();

		resource.check_path_segments_are_the_same(&mut segment_patterns);
	}

	#[test]
	#[should_panic(expected = "is not unique in the path")]
	fn check_names_are_unique_in_the_path1() {
		let mut parent = Resource::new("/abc0/$abc1:@(p)/*abc2");
		let faulty_resource = Resource::new("/$abc1:@cn(p)");

		parent.add_subresource(faulty_resource);
	}

	#[test]
	#[should_panic(expected = "is not unique in the path")]
	fn check_names_are_unique_in_the_path2() {
		let mut parent = Resource::new("/abc0/$abc1:@(p)/*abc2/abc3");
		let mut abc4 = Resource::new("/abc4");
		let faulty_abc2 = Resource::new("/*abc2");
		abc4.add_subresource(faulty_abc2);

		parent.add_subresource(abc4);
	}

	#[test]
	fn add_subresource_under() {
		let mut parent = Resource::new("/abc0_0/*abc1_0");

		struct Case<'a> {
			full_path: &'a str,
			prefix_route_from_parent: &'a str,
			resource_pattern: &'a str,
			route_from_parent: &'a str,
			resource_has_handler: bool,
		}

		let cases = [
			Case {
				full_path: "/abc0_0/*abc1_0/$abc2_0:@(p)/abc3_0",
				prefix_route_from_parent: "/$abc2_0:@(p)",
				resource_pattern: "abc3_0",
				route_from_parent: "/$abc2_0:@(p)/abc3_0",
				resource_has_handler: true,
			},
			Case {
				full_path: "/abc0_0/*abc1_0/$abc2_0:@(p)/abc3_0/*abc4_0",
				prefix_route_from_parent: "/$abc2_0:@(p)/",
				resource_pattern: "*abc4_0",
				route_from_parent: "/$abc2_0:@(p)/abc3_0/*abc4_0",
				resource_has_handler: false,
			},
			Case {
				full_path: "/abc0_0/*abc1_0/$abc2_0:@(p)/abc3_0/abc4_1",
				prefix_route_from_parent: "",
				resource_pattern: "abc4_1",
				route_from_parent: "/$abc2_0:@(p)/abc3_0/abc4_1",
				resource_has_handler: true,
			},
			Case {
				full_path: "/abc0_0/*abc1_0/*abc2_1/abc3_0",
				prefix_route_from_parent: "",
				resource_pattern: "abc3_0",
				route_from_parent: "/*abc2_1/abc3_0",
				resource_has_handler: false,
			},
			Case {
				full_path: "/abc0_0/*abc1_0/*abc2_1/abc3_0/$abc4_0:@cn(p)/",
				prefix_route_from_parent: "/*abc2_1/abc3_0",
				resource_pattern: "$abc4_0:@cn(p)",
				route_from_parent: "/*abc2_1/abc3_0/$abc4_0:@cn(p)",
				resource_has_handler: true,
			},
			Case {
				full_path: "/abc0_0/*abc1_0/*abc2_1/abc3_0/$abc4_1:@cn(p)/",
				prefix_route_from_parent: "/*abc2_1/abc3_0",
				resource_pattern: "$abc4_1:@cn(p)",
				route_from_parent: "/*abc2_1/abc3_0/$abc4_1:@cn(p)",
				resource_has_handler: false,
			},
			Case {
				full_path: "/abc0_0/*abc1_0/*abc2_1/abc3_0/$abc4_1:@cn(p)/abc5_0",
				prefix_route_from_parent: "/*abc2_1/abc3_0/$abc4_1:@cn(p)",
				resource_pattern: "abc5_0",
				route_from_parent: "/*abc2_1/abc3_0/$abc4_1:@cn(p)/abc5_0",
				resource_has_handler: false,
			},
			Case {
				full_path: "/abc0_0/*abc1_0/*abc2_1/$abc3_1:@(p)",
				prefix_route_from_parent: "/*abc2_1",
				resource_pattern: "$abc3_1:@(p)",
				route_from_parent: "/*abc2_1/$abc3_1:@(p)",
				resource_has_handler: false,
			},
		];

		for case in cases.iter() {
			let new_resource = Resource::new(case.full_path);

			parent.add_subresource_under(case.prefix_route_from_parent, new_resource);
			let (resource, _) = parent.leaf_resource_mut(RouteSegments::new(case.route_from_parent));

			assert_eq!(
				resource
					.pattern
					.compare(&Pattern::parse(case.resource_pattern)),
				Similarity::Same
			);

			if case.resource_has_handler {
				resource.set_handler_for(get(DummyHandler::<DefaultResponseFuture>::new()));
			}

			resource.check_path_segments_are_the_same(&mut route_to_patterns(case.full_path).into_iter());
		}

		{
			let mut resource3_0 = Resource::new("/abc3_0");
			resource3_0
				.subresource_mut("/*abc4_0")
				.set_handler_for(post(DummyHandler::<DefaultResponseFuture>::new()));

			let resource4_1 = Resource::new("/abc4_2");
			resource3_0.add_subresource_under("", resource4_1);

			parent.add_subresource_under(cases[0].prefix_route_from_parent, resource3_0);

			let (resource3_0, _) = parent.leaf_resource(RouteSegments::new(cases[0].route_from_parent));
			assert_eq!(resource3_0.static_resources.len(), 2);
			assert_eq!(resource3_0.method_handlers.count(), 1);

			let (resource4_0, _) = parent.leaf_resource(RouteSegments::new(cases[1].route_from_parent));
			assert_eq!(resource4_0.method_handlers.count(), 1);

			let (resource4_1, _) = parent.leaf_resource(RouteSegments::new(cases[2].route_from_parent));
			assert_eq!(resource4_1.method_handlers.count(), 1);
		}

		{
			let mut resource2_1 = Resource::new("/abc0_0/*abc1_0/*abc2_1");
			resource2_1.set_handler_for(get(DummyHandler::<DefaultResponseFuture>::new()));

			let mut resource4_0 = Resource::new("/$abc4_0:@cn(p)");
			resource4_0
				.subresource_mut("/*abc5_0")
				.set_handler_for(get(DummyHandler::<DefaultResponseFuture>::new()));
			resource2_1.add_subresource_under("/abc3_0", resource4_0);

			let mut resource4_1 = Resource::new("/$abc4_1:@cn(p)/");
			resource4_1.set_handler_for(put(DummyHandler::<DefaultResponseFuture>::new()));
			resource2_1.add_subresource_under("/abc3_0", resource4_1);

			let resource5_0 = Resource::new("/abc0_0/*abc1_0/*abc2_1/abc3_0/*abc4_2/$abc5_0:@(p)");
			resource2_1.add_subresource_under("/abc3_0", resource5_0);

			parent.add_subresource_under("", resource2_1);

			let (resource2_1, _) = parent.leaf_resource(RouteSegments::new("/*abc2_1"));
			assert_eq!(resource2_1.static_resources.len(), 1);
			assert_eq!(resource2_1.regex_resources.len(), 1);
			assert_eq!(resource2_1.method_handlers.count(), 1);

			let (resource4_0, _) =
				resource2_1.leaf_resource(RouteSegments::new("/abc3_0/$abc4_0:@cn(p)"));
			assert!(resource4_0.some_wildcard_resource.is_some());
			assert_eq!(resource4_0.method_handlers.count(), 1);

			let (resource4_1, _) =
				resource2_1.leaf_resource(RouteSegments::new("/abc3_0/$abc4_1:@cn(p)"));
			assert_eq!(resource4_1.static_resources.len(), 1);
			assert_eq!(resource4_1.method_handlers.count(), 1);

			let (resource4_2, _) = resource2_1.leaf_resource(RouteSegments::new("/abc3_0/*abc4_2"));
			assert_eq!(resource4_2.regex_resources.len(), 1);
		}
	}
}
