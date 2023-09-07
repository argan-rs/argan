use std::{
	any::{Any, TypeId},
	fmt::{Debug, Display},
	sync::Arc,
};

use crate::{
	body::IncomingBody,
	handler::{
		request_handlers::MethodHandlers, wrap_arc_handler, AdaptiveHandler, ArcHandler, Handler,
		IntoArcHandler, IntoHandler,
	},
	middleware::{IntoResponseAdapter, Layer, ResponseFutureBoxer},
	pattern::{patterns_to_string, Pattern, Similarity},
	request::Request,
	response::{IntoResponse, Response},
	routing::{Method, RouteSegments},
	utils::{BoxedError, BoxedFuture},
};

// --------------------------------------------------

mod futures;
mod service;

use self::{
	futures::{RequestPasserFuture, RequestReceiverFuture},
	service::{request_handler, request_passer, request_receiver},
};

pub use service::ResourceService;

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

pub struct Resource {
	pattern: Pattern,
	prefix_path_patterns: Vec<Pattern>,

	static_resources: Vec<Resource>,
	regex_resources: Vec<Resource>,
	wildcard_resource: Option<Box<Resource>>,

	request_receiver: Option<ArcHandler>,
	request_passer: Option<ArcHandler>,
	request_handler: Option<ArcHandler>,

	method_handlers: MethodHandlers,

	states: Vec<Box<dyn Any + Send + Sync>>,

	// TODO: configs, state, redirect, parent
	is_subtree_handler: bool,
}

// -------------------------

impl Resource {
	pub fn new(path_patterns: &str) -> Resource {
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

		let mut resource_pattern: Pattern;
		let mut prefix_path_patterns = Vec::new();

		let resource_pattern = loop {
			let (route_segment, _) = route_segments.next().unwrap();
			let pattern = Pattern::parse(route_segment);

			if route_segments.has_remaining_segments() {
				prefix_path_patterns.push(pattern);

				continue;
			}

			break pattern;
		};

		Self::with_prefix_path_patterns(prefix_path_patterns, resource_pattern)
	}

	#[inline]
	pub(crate) fn with_pattern_str(pattern: &str) -> Resource {
		let pattern = Pattern::parse(pattern);

		Self::with_pattern(pattern)
	}

	#[inline]
	pub(crate) fn with_pattern(pattern: Pattern) -> Resource {
		Self::with_prefix_path_patterns(Vec::new(), pattern)
	}

	#[inline]
	pub(crate) fn with_prefix_path_patterns(
		prefix_path_patterns: Vec<Pattern>,
		resource_pattern: Pattern,
	) -> Resource {
		if let Pattern::Regex(ref name, None) = resource_pattern {
			panic!("pattern has no regex segment")
		}

		Resource {
			pattern: resource_pattern,
			prefix_path_patterns,
			static_resources: Vec::new(),
			regex_resources: Vec::new(),
			wildcard_resource: None,
			request_receiver: None,
			request_passer: None,
			request_handler: None,
			method_handlers: MethodHandlers::new(),
			states: Vec::new(),
			is_subtree_handler: false,
		}
	}

	// -------------------------

	#[inline]
	fn name(&self) -> Option<&str> {
		self.pattern.name()
	}

	#[inline]
	pub fn is_subtree_handler(&self) -> bool {
		self.is_subtree_handler
	}

	#[inline]
	pub fn can_handle_request(&self) -> bool {
		!self.method_handlers.is_empty()
	}

	#[inline]
	fn has_some_effect(&self) -> bool {
		self.method_handlers.has_some_effect()
			|| self.request_handler.is_some()
			|| self.request_passer.is_some()
			|| self.request_receiver.is_some()
	}

	// -------------------------

	pub fn add_subresource(&mut self, mut new_resource: Resource) {
		if !new_resource.prefix_path_patterns.is_empty() {
			let mut prefix_path_patterns =
				std::mem::take(&mut new_resource.prefix_path_patterns).into_iter();

			self.check_path_segments_are_the_same(&mut prefix_path_patterns);

			if prefix_path_patterns.len() > 0 {
				let subresource_to_be_parent = self.by_patterns_subresource_mut(prefix_path_patterns);
				subresource_to_be_parent.add_subresource(new_resource);

				return;
			}
		};

		self.check_names_are_unique_in_the_path(&new_resource);

		// -----

		macro_rules! add_resource {
			($resources:expr, $new_resource:ident) => {
				if let Some(position) = $resources.iter_mut().position(
					|resource| resource.pattern.compare(&$new_resource.pattern) == Similarity::Same
				) {
					let dummy_resource = Resource::with_pattern_str("dummy"); // TODO: Provide default constructor.
					let mut existing_resource = std::mem::replace(&mut $resources[position], dummy_resource);

					if !$new_resource.has_some_effect() {
						existing_resource.keep_subresources($new_resource);
					} else if !existing_resource.has_some_effect() {
						$new_resource.prefix_path_patterns = std::mem::take(&mut existing_resource.prefix_path_patterns);
						$new_resource.keep_subresources(existing_resource);
						existing_resource = $new_resource;
					} else {
						// TODO: Improve the error message.
						panic!("sub resource with the same pattern exists")
					}

					$resources[position] = existing_resource;
				} else {
					$new_resource.prefix_path_patterns = self.path_patterns();
					$resources.push($new_resource);
				}
			}
		}

		// -----

		match &new_resource.pattern {
			Pattern::Static(_) => add_resource!(self.static_resources, new_resource),
			Pattern::Regex(..) => add_resource!(self.regex_resources, new_resource),
			Pattern::Wildcard(_) => {
				if let Some(mut wildcard_resource) = self.wildcard_resource.take() {
					if wildcard_resource.pattern.compare(&new_resource.pattern) == Similarity::Same {
						if !new_resource.has_some_effect() {
							wildcard_resource.keep_subresources(new_resource);
						} else if !wildcard_resource.has_some_effect() {
							new_resource.prefix_path_patterns =
								std::mem::take(&mut wildcard_resource.prefix_path_patterns);
							new_resource.keep_subresources(*wildcard_resource);
							*wildcard_resource = new_resource;
						}
					} else {
						// TODO: Improve the error message.
						panic!("resource can have only one child resource with a wildcard pattern")
					}

					self.wildcard_resource = Some(wildcard_resource);
				} else {
					new_resource.prefix_path_patterns = self.path_patterns();
					self.wildcard_resource = Some(Box::new(new_resource));
				}
			}
		}
	}

	#[inline]
	fn check_path_segments_are_the_same(
		&self,
		prefix_segment_patterns: &mut impl Iterator<Item = Pattern>,
	) {
		let self_path_patterns = self
			.prefix_path_patterns
			.iter()
			.chain(std::iter::once(&self.pattern));
		for self_path_segment_pattern in self_path_patterns {
			let Some(prefix_path_segment_pattern) = prefix_segment_patterns.next() else {
				panic!("prefix path patterns must be the same with the path patterns of the parent")
			};

			if let Pattern::Regex(prefix_path_segment_name, None) = &prefix_path_segment_pattern {
				if let Pattern::Regex(self_path_segment_name, _) = self_path_segment_pattern {
					if self_path_segment_name == prefix_path_segment_name {
						continue;
					}
				}

				panic!(
					"no prefix path segment resource '${}' exists",
					prefix_path_segment_name
				)
			}

			if self_path_segment_pattern.compare(&prefix_path_segment_pattern) != Similarity::Same {
				panic!(
					"no prefix path segment resource '${}' exists",
					prefix_path_segment_pattern
				)
			}
		}
	}

	#[inline]
	fn by_patterns_subresource_mut(
		&mut self,
		mut patterns: impl Iterator<Item = Pattern>,
	) -> &mut Resource {
		let (mut leaf_resource_in_the_path, patterns) = self.by_patterns_leaf_resource_mut(patterns);
		leaf_resource_in_the_path.by_patterns_new_subresource_mut(patterns)
	}

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
							// Unwrap safety: Regex resources must have a name.
							resource.name().unwrap() == name.as_ref()
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
						.wildcard_resource
						.as_ref()
						.is_some_and(|resource| resource.pattern.compare(pattern) == Similarity::Same)
					{
						leaf_resource = leaf_resource.wildcard_resource.as_deref_mut().unwrap();
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

	#[inline]
	fn check_names_are_unique_in_the_path(&self, new_resource: &Resource) {
		if new_resource
			.name()
			.is_some_and(|name| self.path_has_the_same_name(name))
		{
			panic!("some resources have the same names in the path");
		}

		let mut resources = Vec::new();
		resources.extend(new_resource.regex_resources.iter());

		if let Some(wildcard_resource) = &new_resource.wildcard_resource {
			resources.push(wildcard_resource);
		}

		loop {
			let Some(resource) = resources.pop() else {
				return;
			};

			// Regex and wildcard resources all must have a name.
			// If the following unwrap() panics then we have a bug in our resource initialization logic.
			let name = resource.name().unwrap();
			if self.path_has_the_same_name(name) {
				panic!("some resources have the same names in the path");
			}

			resources.extend(resource.regex_resources.iter());

			if let Some(wildcard_resource) = &resource.wildcard_resource {
				resources.push(wildcard_resource);
			}
		}
	}

	fn keep_subresources(&mut self, mut other: Resource) {
		macro_rules! keep_other_resources {
			(mut $resources:expr, mut $other_resources:expr) => {
				if !$other_resources.is_empty() {
					if $resources.is_empty() {
						for mut other_resource in $other_resources.iter_mut() {
							other_resource.prefix_path_patterns = self.path_patterns();
						}

						$resources = $other_resources;
					} else {
						for mut other_resource in $other_resources {
							if let Some(position) = $resources.iter().position(
								|resource| resource.pattern.compare(&other_resource.pattern) == Similarity::Same
							) {
								let dummy_resource = Resource::with_pattern_str("dummy"); // TODO: Provide default constructor;
								let mut resource = std::mem::replace(&mut $resources[position], dummy_resource);

								if !other_resource.has_some_effect() {
									resource.keep_subresources(other_resource);
								} else if !resource.has_some_effect() {
									other_resource.prefix_path_patterns = std::mem::take(&mut resource.prefix_path_patterns);
									other_resource.keep_subresources(resource);
									resource = other_resource;
								} else {
									// TODO: Improve error message.
									panic!("sub resources has duplicate pattern")
								}

								$resources[position] = resource;
							} else {
								other_resource.prefix_path_patterns = self.path_patterns();
								$resources.push(other_resource);
							}
						}
					}
				}
			}
		}

		// -----

		keep_other_resources!(mut self.static_resources, mut other.static_resources);

		keep_other_resources!(mut self.regex_resources, mut other.regex_resources);

		if let Some(mut other_wildcard_resource) = other.wildcard_resource.take() {
			if let Some(mut wildcard_resource) = self.wildcard_resource.take() {
				if wildcard_resource
					.pattern
					.compare(&other_wildcard_resource.pattern)
					== Similarity::Same
				{
					if !other_wildcard_resource.has_some_effect() {
						wildcard_resource.keep_subresources(*other_wildcard_resource);
					} else if !wildcard_resource.has_some_effect() {
						other_wildcard_resource.prefix_path_patterns =
							std::mem::take(&mut wildcard_resource.prefix_path_patterns);
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

				self.wildcard_resource = Some(wildcard_resource);
			} else {
				other_wildcard_resource.prefix_path_patterns = self.path_patterns();
				self.wildcard_resource = Some(other_wildcard_resource);
			}
		}
	}

	#[inline]
	fn path_patterns(&self) -> Vec<Pattern> {
		let mut prefix_patterns = self.prefix_path_patterns.clone();
		prefix_patterns.push(self.pattern.clone());

		prefix_patterns
	}

	pub fn add_subresource_under(&mut self, route: &str, mut new_resource: Resource) {
		if !new_resource.prefix_path_patterns.is_empty() {
			let mut new_resource_prefix_path_patterns =
				std::mem::take(&mut new_resource.prefix_path_patterns).into_iter();

			self.check_path_segments_are_the_same(&mut new_resource_prefix_path_patterns);

			if route.is_empty() {
				if new_resource_prefix_path_patterns.len() > 0 {
					let subresource_to_be_parent =
						self.by_patterns_subresource_mut(new_resource_prefix_path_patterns);
					subresource_to_be_parent.add_subresource(new_resource);
				} else {
					self.add_subresource(new_resource);
				}

				return;
			}

			let mut prefix_route_patterns = Vec::new();

			let prefix_route_segments = RouteSegments::new(route);
			for (prefix_route_segment, _) in prefix_route_segments {
				let Some(prefix_path_segment_pattern) = new_resource_prefix_path_patterns.next() else {
					panic!("there are fewer path segments in the resource's own prefix path than the path it's being added")
				};

				let prefix_route_segment_pattern = Pattern::parse(prefix_route_segment);
				if let Pattern::Regex(ref prefix_route_segment_name, None) = prefix_route_segment_pattern {
					if let Pattern::Regex(ref prefix_path_segment_name, _) = prefix_path_segment_pattern {
						if prefix_path_segment_name == prefix_route_segment_name {
							prefix_route_patterns.push(prefix_path_segment_pattern);

							continue;
						}
					}
				} else if let Pattern::Regex(ref prefix_path_segment_name, None) =
					prefix_path_segment_pattern
				{
					if let Pattern::Regex(ref prefix_route_segment_name, _) = prefix_route_segment_pattern {
						if prefix_route_segment_name == prefix_path_segment_name {
							prefix_route_patterns.push(prefix_route_segment_pattern);

							continue;
						}
					}
				}

				if prefix_route_segment_pattern.compare(&prefix_path_segment_pattern) != Similarity::Same {
					panic!("prefix segments' patterns must be the same with the segment patterns of the parent resources")
				}

				prefix_route_patterns.push(prefix_route_segment_pattern);
			}

			let mut subresource_to_be_parent =
				self.by_patterns_subresource_mut(prefix_route_patterns.into_iter());
			if new_resource_prefix_path_patterns.len() > 0 {
				subresource_to_be_parent =
					subresource_to_be_parent.by_patterns_subresource_mut(new_resource_prefix_path_patterns);
			}

			subresource_to_be_parent.add_subresource(new_resource);

			return;
		}

		if route.is_empty() {
			self.add_subresource(new_resource);
		} else {
			let subresource_to_be_parent = self.subresource_mut(route);
			subresource_to_be_parent.add_subresource(new_resource);
		}
	}

	pub fn subresource_mut(&mut self, route: &str) -> &mut Resource {
		if route.is_empty() {
			panic!("empty route")
		}

		if route == "/" {
			panic!("root cannot be a sub-resource")
		}

		if !route.starts_with('/') {
			panic!("{} route must start with '/'", route)
		}

		let mut segments = RouteSegments::new(route);
		let (mut leaf_resource_in_the_path, segments) = self.leaf_resource_mut(segments);

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
				Pattern::Regex(ref name, ref some_regex) => {
					let some_position = leaf_resource.regex_resources.iter().position(|resource| {
						if some_regex.is_some() {
							resource.pattern.compare(&pattern) == Similarity::Same
						} else {
							// Unwrap safety: Regex resources must have a name.
							resource.name().unwrap() == name.as_ref()
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
						.wildcard_resource
						.as_ref()
						.is_some_and(|resource| resource.pattern.compare(&pattern) == Similarity::Same)
					{
						leaf_resource = leaf_resource.wildcard_resource.as_deref().unwrap();
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
				Pattern::Regex(ref name, ref some_regex) => {
					let some_position = leaf_resource.regex_resources.iter().position(|resource| {
						if some_regex.is_some() {
							resource.pattern.compare(&pattern) == Similarity::Same
						} else {
							// Unwrap safety: Regex resources must have a name.
							resource.name().unwrap() == name.as_ref()
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
						.wildcard_resource
						.as_ref()
						.is_some_and(|resource| resource.pattern.compare(&pattern) == Similarity::Same)
					{
						leaf_resource = leaf_resource.wildcard_resource.as_deref_mut().unwrap();
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

		for prefix_pattern in self.prefix_path_patterns.iter() {
			if let Some(pattern_name) = prefix_pattern.name() {
				if pattern_name == name {
					return true;
				}
			}
		}

		false
	}

	// -------------------------

	// TODO: Remove BoxedErrors.
	pub fn set_state<S: Clone + Send + Sync + 'static>(
		&mut self,
		state: S,
	) -> Result<(), BoxedError> {
		let state_type_id = state.type_id();

		if self
			.states
			.iter()
			.any(|existing_state| (*existing_state).type_id() == state_type_id)
		{
			return Err(BoxedError::from(format!(
				"resource already has a state of type '{:?}'",
				TypeId::of::<S>()
			)));
		}

		self.states.push(Box::new(state));

		Ok(())
	}

	pub fn state<S: Clone + Send + Sync + 'static>(&self) -> Result<&S, BoxedError> {
		self
			.states
			.iter()
			.find_map(|state| state.downcast_ref::<S>())
			.ok_or_else(|| {
				BoxedError::from(format!(
					"resource has no state of type '{:?}'",
					TypeId::of::<S>()
				))
			})
	}

	// pub fn set_config(&mut self, config: Config) {
	// 	todo!()
	// }
	//
	// pub fn config(&self) -> Result<Config, BoxedError> {
	// 	todo!()
	// }

	// TODO: Create IntoMethod sealed trait and implement it for a Method and String.
	pub fn set_handler<H, M>(&mut self, method: Method, handler: H)
	where
		H: IntoHandler<M, IncomingBody>,
		H::Handler: Handler + Send + Sync + 'static,
		<H::Handler as Handler>::Response: IntoResponse,
	{
		let ready_handler =
			ResponseFutureBoxer::wrap(IntoResponseAdapter::wrap(handler.into_handler()));
		self
			.method_handlers
			.set_handler(method, ready_handler.into_arc_handler())
	}

	pub fn wrap_request_receiver<L, LayeredB>(&mut self, layer: L)
	where
		L: Layer<AdaptiveHandler<LayeredB>, LayeredB>,
		<L>::Handler: Handler + Send + Sync + 'static,
		<L::Handler as Handler>::Response: IntoResponse,
	{
		let boxed_request_receiver = match self.request_receiver.take() {
			Some(request_receiver) => request_receiver,
			None => {
				let request_receiver = <fn(Request) -> RequestReceiverFuture as IntoHandler<(
					(),
					Request,
				)>>::into_handler(request_receiver);

				ResponseFutureBoxer::wrap(request_receiver).into_arc_handler()
			}
		};

		let boxed_request_receiver = wrap_arc_handler(boxed_request_receiver, layer);

		self.request_receiver.replace(boxed_request_receiver);
	}

	pub fn wrap_request_passer<L, LayeredB>(&mut self, layer: L)
	where
		L: Layer<AdaptiveHandler<LayeredB>, LayeredB>,
		<L>::Handler: Handler + Send + Sync + 'static,
		<L::Handler as Handler>::Response: IntoResponse,
	{
		let boxed_request_passer = match self.request_passer.take() {
			Some(request_passer) => request_passer,
			None => {
				let request_passer =
					<fn(Request) -> RequestPasserFuture as IntoHandler<((), Request)>>::into_handler(
						request_passer,
					);

				ResponseFutureBoxer::wrap(request_passer.into_handler()).into_arc_handler()
			}
		};

		let boxed_request_passer = wrap_arc_handler(boxed_request_passer, layer);

		self.request_passer.replace(boxed_request_passer);
	}

	pub fn wrap_request_handler<L, LayeredB>(&mut self, layer: L)
	where
		L: Layer<AdaptiveHandler<LayeredB>, LayeredB>,
		<L>::Handler: Handler + Send + Sync + 'static,
		<L::Handler as Handler>::Response: IntoResponse,
	{
		let boxed_request_handler = match self.request_handler.take() {
			Some(request_handler) => request_handler,
			None => {
				let request_handler =
					<fn(Request) -> BoxedFuture<Response> as IntoHandler<()>>::into_handler(request_handler);

				request_handler.into_arc_handler()
			}
		};

		let boxed_request_handler = wrap_arc_handler(boxed_request_handler, layer);

		self.request_handler.replace(boxed_request_handler);
	}

	pub fn wrap_method_handler<L, LayeredB>(&mut self, method: Method, layer: L)
	where
		L: Layer<AdaptiveHandler<LayeredB>, LayeredB>,
		<L>::Handler: Handler + Send + Sync + 'static,
		<L::Handler as Handler>::Response: IntoResponse,
	{
		self.method_handlers.wrap_handler(method, layer);
	}

	// -------------------------

	pub fn set_subresource_state<S: Clone + Send + Sync + 'static>(&mut self, route: &str, state: S) {
		let subresource = self.subresource_mut(route);
		subresource.set_state(state);
	}

	pub fn subresource_state<S: Clone + Send + Sync + 'static>(
		&self,
		route: &str,
	) -> Result<&S, BoxedError> {
		let mut route_segments = RouteSegments::new(route);
		let (subresource, route_segments) = self.leaf_resource(route_segments);

		if route_segments.has_remaining_segments() {
			return Err(format!("{} doesn't exist", route).into());
		}

		subresource.state()
	}

	// pub fn set_subresource_config(&mut self, route: &str, config: Config) {
	// 	let subresource = self.subresource_mut(route);
	// 	subresource.set_config(config);
	// }
	//
	// pub fn subresource_config(&self, route: &str) -> Result<Config, BoxedError> {
	// 	let mut route_segments = RouteSegments::new(route);
	// 	let (subresource, route_segments) = self.leaf_resource(route_segments);
	//
	// 	if route_segments.has_remaining_segments() {
	// 		return Err(format!("{} doesn't exist", route).into());
	// 	}
	//
	// 	subresource.config()
	// }

	pub fn set_subresource_handler<H, M>(&mut self, route: &str, method: Method, handler: H)
	where
		H: IntoHandler<M, IncomingBody>,
		H::Handler: Handler + Send + Sync + 'static,
		<H::Handler as Handler>::Response: IntoResponse,
	{
		let subresource = self.subresource_mut(route);
		subresource.set_handler(method, handler);
	}

	pub fn wrap_subresource_request_receiver<L, LayeredB>(&mut self, route: &str, layer: L)
	where
		L: Layer<AdaptiveHandler<LayeredB>, LayeredB>,
		L::Handler: Handler + Send + Sync + 'static,
		<L::Handler as Handler>::Response: IntoResponse,
	{
		let subresource = self.subresource_mut(route);
		subresource.wrap_request_receiver(layer);
	}

	pub fn wrap_subresource_request_passer<L, LayeredB>(&mut self, route: &str, layer: L)
	where
		L: Layer<AdaptiveHandler<LayeredB>, LayeredB>,
		L::Handler: Handler + Send + Sync + 'static,
		<L::Handler as Handler>::Response: IntoResponse,
	{
		let subresource = self.subresource_mut(route);
		subresource.wrap_request_passer(layer);
	}

	pub fn wrap_subresource_request_handler<L, LayeredB>(&mut self, route: &str, layer: L)
	where
		L: Layer<AdaptiveHandler<LayeredB>, LayeredB>,
		L::Handler: Handler + Send + Sync + 'static,
		<L::Handler as Handler>::Response: IntoResponse,
	{
		let subresource = self.subresource_mut(route);
		subresource.wrap_request_handler(layer);
	}

	pub fn wrap_subresource_method_handler<L, LayeredB>(
		&mut self,
		route: &str,
		method: Method,
		layer: L,
	) where
		L: Layer<AdaptiveHandler<LayeredB>, LayeredB>,
		L::Handler: Handler + Send + Sync + 'static,
		<L::Handler as Handler>::Response: IntoResponse,
	{
		let subresource = self.subresource_mut(route);
		subresource.wrap_method_handler(method, layer);
	}

	// -------------------------

	pub fn set_subresources_state<S: Clone + Send + Sync + 'static>(&mut self, state: S) {
		self.call_for_each_subresource(|subresource| {
			subresource.set_state(state.clone());
		});
	}

	// pub fn set_subresources_config(&mut self, config: Config) {
	// 	todo!()
	// }

	pub fn wrap_subresources_request_receivers<L, LayeredB>(&mut self, layer: L)
	where
		L: Layer<AdaptiveHandler<LayeredB>, LayeredB> + Clone,
		L::Handler: Handler + Send + Sync + 'static,
		<L::Handler as Handler>::Response: IntoResponse,
	{
		self.call_for_each_subresource(|subresource| subresource.wrap_request_receiver(layer.clone()))
	}

	pub fn wrap_subresources_request_passers<L, LayeredB>(&mut self, layer: L)
	where
		L: Layer<AdaptiveHandler<LayeredB>, LayeredB> + Clone,
		L::Handler: Handler + Send + Sync + 'static,
		<L::Handler as Handler>::Response: IntoResponse,
	{
		self.call_for_each_subresource(|subresource| subresource.wrap_request_passer(layer.clone()))
	}

	pub fn wrap_subresources_request_handlers<L, LayeredB>(&mut self, layer: L)
	where
		L: Layer<AdaptiveHandler<LayeredB>, LayeredB> + Clone,
		L::Handler: Handler + Send + Sync + 'static,
		<L::Handler as Handler>::Response: IntoResponse,
	{
		self.call_for_each_subresource(|subresource| subresource.wrap_request_handler(layer.clone()))
	}

	pub fn wrap_subresources_method_handlers<L, LayeredB>(&mut self, method: Method, layer: L)
	where
		L: Layer<AdaptiveHandler<LayeredB>, LayeredB> + Clone,
		L::Handler: Handler + Send + Sync + 'static,
		<L::Handler as Handler>::Response: IntoResponse,
	{
		self.call_for_each_subresource(|subresource| {
			subresource.wrap_method_handler(method.clone(), layer.clone())
		})
	}

	fn call_for_each_subresource(&mut self, func: impl Fn(&mut Resource)) {
		let mut subresources = Vec::new();
		subresources.extend(self.static_resources.iter_mut());
		subresources.extend(self.regex_resources.iter_mut());
		if let Some(resource) = self.wildcard_resource.as_deref_mut() {
			subresources.push(resource);
		}

		for i in 0.. {
			let Some(subresource) = subresources.pop() else {
				break;
			};

			func(subresource);

			subresources.extend(subresource.static_resources.iter_mut());
			subresources.extend(subresource.regex_resources.iter_mut());
			if let Some(resource) = subresource.wildcard_resource.as_deref_mut() {
				subresources.push(resource);
			}
		}
	}

	pub fn into_service(self) -> ResourceService {
		let Resource {
			pattern,
			static_resources,
			regex_resources,
			wildcard_resource,
			request_receiver,
			request_passer,
			request_handler,
			method_handlers,
			states: state,
			is_subtree_handler,
			..
		} = self;

		let static_resources = static_resources
			.into_iter()
			.map(|resource| resource.into_service())
			.collect::<Arc<[ResourceService]>>();

		let regex_resources = regex_resources
			.into_iter()
			.map(|resource| resource.into_service())
			.collect::<Arc<[ResourceService]>>();

		let wildcard_resource = wildcard_resource.map(|resource| Arc::new(resource.into_service()));

		ResourceService {
			pattern,
			static_resources,
			regex_resources,
			wildcard_resource,
			request_receiver,
			request_passer,
			request_handler,
			method_handlers,
			state: Arc::from(state),
			is_subtree_handler,
		}
	}
}

impl Display for Resource {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(
			f,
			"{}/{}",
			patterns_to_string(&self.prefix_path_patterns),
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
				prefix_path_patterns: {},
				static_resources_count: {},
				regex_resources_count: {},
				wildcard_resource_exists: {},
				layered_request_receiver: {},
				layered_request_passer: {},
				layered_request_handler: {},
				method_handlers: {{ count: {}, unsupported_method_handler_exists: {} }},
				states_count: {},
				is_subtree_handler: {},
			}}",
			&self.pattern,
			patterns_to_string(&self.prefix_path_patterns),
			self.static_resources.len(),
			self.regex_resources.len(),
			self.wildcard_resource.is_some(),
			self.request_receiver.is_some(),
			self.request_passer.is_some(),
			self.request_handler.is_some(),
			self.method_handlers.count(),
			self
				.method_handlers
				.has_layered_unsupported_method_handler(),
			self.states.len(),
			self.is_subtree_handler,
		)
	}
}

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

#[cfg(test)]
mod test {
	use crate::{
		handler::{futures::DefaultResponseFuture, DummyHandler},
		pattern::string_to_patterns,
	};

	use super::*;

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
				parent.by_patterns_leaf_resource_mut(string_to_patterns(case.1).into_iter());
			resource.check_path_segments_are_the_same(&mut string_to_patterns(case.0).into_iter());
		}

		{
			// Existing resources in the tree.

			let (resource2_0, _) = parent.leaf_resource_mut(RouteSegments::new("/$abc2_0:@(p0)"));
			resource2_0.set_handler(Method::POST, DummyHandler::<DefaultResponseFuture>::new());
			resource2_0.set_subresource_handler(
				"/$abc3_1:@cn0(p0)/*abc4_0",
				Method::GET,
				DummyHandler::<DefaultResponseFuture>::new(),
			);

			let (resource4_2, _) =
				resource2_0.leaf_resource_mut(RouteSegments::new("/$abc3_1:@cn0(p0)/abc4_2"));
			resource4_2.new_subresource_mut(RouteSegments::new("/abc5_0"));
		}

		{
			// New resources.

			let mut resource2_0 = Resource::new("/$abc2_0:@(p0)");

			let mut resource3_1 = Resource::new("/$abc3_1:@cn0(p0)");
			resource3_1.set_handler(Method::GET, DummyHandler::<DefaultResponseFuture>::new());
			resource3_1.set_handler(Method::POST, DummyHandler::<DefaultResponseFuture>::new());
			resource3_1.set_subresource_handler(
				"/abc4_1",
				Method::POST,
				DummyHandler::<DefaultResponseFuture>::new(),
			);
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
			assert!(resource3_1.wildcard_resource.is_some());
			assert_eq!(resource3_1.method_handlers.count(), 2);

			let (resource4_0, _) = resource3_1.leaf_resource(RouteSegments::new("/*abc4_0"));
			assert_eq!(resource4_0.method_handlers.count(), 1);

			let (resource4_2, _) = resource3_1.leaf_resource(RouteSegments::new("/abc4_2"));
			assert_eq!(resource4_2.static_resources.len(), 1);

			let (resource5_0, _) = resource4_2.leaf_resource(RouteSegments::new("/abc5_0"));
			resource5_0.check_path_segments_are_the_same(
				&mut string_to_patterns("/abc0_0/*abc1_0/$abc2_0:@(p0)/$abc3_1:@cn0(p0)/abc4_2/abc5_0")
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
			resource3_0.set_handler(Method::GET, DummyHandler::<DefaultResponseFuture>::new());

			parent.add_subresource(resource3_0);
			let (resource3_0, _) = parent.leaf_resource_mut(RouteSegments::new(route3_0));
			resource3_0.check_path_segments_are_the_same(&mut string_to_patterns(pattern3_0).into_iter());
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
			let mut segmets = RouteSegments::new(segment_patterns.1);
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
	#[should_panic(expected = "no prefix path segment resource")]
	fn check_path_segments_are_the_same_panic3() {
		let resource = Resource::new("/news/$area:@(local|worldwide)");
		let mut segment_patterns = vec![Pattern::parse("news"), Pattern::parse("*area")].into_iter();

		resource.check_path_segments_are_the_same(&mut segment_patterns);
	}

	#[test]
	#[should_panic(expected = "the same name")]
	fn check_names_are_unique_in_the_path1() {
		let mut parent = Resource::new("/abc0/$abc1:@(p)/*abc2");
		let mut faulty_resource = Resource::new("/$abc1:@cn(p)");

		parent.add_subresource(faulty_resource);
	}

	#[test]
	#[should_panic(expected = "the same name")]
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
			let (mut resource, _) = parent.leaf_resource_mut(RouteSegments::new(case.route_from_parent));

			assert_eq!(
				resource
					.pattern
					.compare(&Pattern::parse(case.resource_pattern)),
				Similarity::Same
			);

			if case.resource_has_handler {
				resource.set_handler(Method::GET, DummyHandler::<DefaultResponseFuture>::new());
			}

			resource
				.check_path_segments_are_the_same(&mut string_to_patterns(case.full_path).into_iter());
		}

		{
			let mut resource3_0 = Resource::new("/abc3_0");
			resource3_0.set_subresource_handler(
				"/*abc4_0",
				Method::POST,
				DummyHandler::<DefaultResponseFuture>::new(),
			);

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
			resource2_1.set_handler(Method::GET, DummyHandler::<DefaultResponseFuture>::new());

			let mut resource4_0 = Resource::new("/$abc4_0:@cn(p)");
			resource4_0.set_subresource_handler(
				"/*abc5_0",
				Method::GET,
				DummyHandler::<DefaultResponseFuture>::new(),
			);
			resource2_1.add_subresource_under("/abc3_0", resource4_0);

			let mut resource4_1 = Resource::new("/$abc4_1:@cn(p)/");
			resource4_1.set_handler(Method::PUT, DummyHandler::<DefaultResponseFuture>::new());
			resource2_1.add_subresource_under("/abc3_0", resource4_1);

			let mut resource5_0 = Resource::new("/abc0_0/*abc1_0/*abc2_1/abc3_0/*abc4_2/$abc5_0:@(p)");
			resource2_1.add_subresource_under("/abc3_0", resource5_0);

			parent.add_subresource_under("", resource2_1);

			let (resource2_1, _) = parent.leaf_resource(RouteSegments::new("/*abc2_1"));
			assert_eq!(resource2_1.static_resources.len(), 1);
			assert_eq!(resource2_1.regex_resources.len(), 1);
			assert_eq!(resource2_1.method_handlers.count(), 1);

			let (resource4_0, _) =
				resource2_1.leaf_resource(RouteSegments::new("/abc3_0/$abc4_0:@cn(p)"));
			assert!(resource4_0.wildcard_resource.is_some());
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
