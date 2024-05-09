//! Resource service types.

// ----------

use std::{
	any,
	fmt::{Debug, Display},
	sync::Arc,
};

use http::Extensions;

use crate::{
	common::{config::ConfigOption, patterns_to_route, IntoArray, SCOPE_VALIDITY},
	handler::{
		kind::HandlerKind,
		request_handlers::{wrap_mistargeted_request_handler, ImplementedMethods, MethodHandlers},
		BoxedHandler,
	},
	middleware::{_request_receiver, targets::LayerTarget},
	pattern::{split_uri_host_and_path, Pattern, Similarity},
	request::ContextProperties,
	routing::RouteSegments,
};

// --------------------------------------------------

mod config;

use self::{
	config::{resource_config_from, ConfigFlags},
	service::{RequestHandler, RequestPasser, RequestReceiver},
};

mod service;
pub use service::{ArcResourceService, LeakedResourceService, ResourceService};

#[cfg(feature = "file-stream")]
mod static_files;

#[cfg(feature = "file-stream")]
pub use static_files::StaticFiles;

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

/// Representation of the web *resource* that corresponds to the path segment component
/// of the URI.
pub struct Resource {
	pattern: Pattern,
	prefix_segment_patterns: Vec<Pattern>,
	some_host_pattern: Option<Pattern>,

	static_resources: Vec<Resource>,
	regex_resources: Vec<Resource>,
	some_wildcard_resource: Option<Box<Resource>>,

	method_handlers: MethodHandlers,
	some_mistargeted_request_handler: Option<BoxedHandler>,

	context_properties: ContextProperties,
	extensions: Extensions,
	middleware: Vec<LayerTarget<Self>>,

	config_flags: ConfigFlags,
}

// -------------------------

impl Resource {
	/// Creates a new `Resource` that corresponds to the last path segment component
	/// of the given URI pattern.
	///
	/// Other components of the URI pattern (host and prefix path segments) will be
	/// used to find the resource's place in the resource tree when it's being added
	/// to another resource.
	///
	/// ```
	/// use argan::Resource;
	///
	/// let mut root = Resource::new("/");
	/// let resource_2 = Resource::new("/resource_1/resource_2");
	///
	/// // When we add `resource_2` to the root, it won't be a direct child of the root.
	/// // Instead, it will be added under `/resource_1`.
	/// root.add_subresource(resource_2);
	/// ```
	pub fn new<P>(uri_pattern: P) -> Resource
	where
		P: AsRef<str>,
	{
		let (some_host_pattern_str, some_path_pattern_str) =
			split_uri_host_and_path(uri_pattern.as_ref());

		let some_host_pattern = some_host_pattern_str.map(|host_pattern_str| {
			let host_pattern = Pattern::parse(host_pattern_str);
			if host_pattern.is_wildcard() {
				panic!("host pattern cannot be a wildcard")
			}

			host_pattern
		});

		let Some(path_pattern_str) = some_path_pattern_str else {
			panic!("empty path pattern")
		};

		let (config_flags, path_pattern_str) = resource_config_from(path_pattern_str);

		if path_pattern_str == "/" {
			let pattern = Pattern::parse(path_pattern_str);

			return Self::with_uri_pattern_and_config_flags(
				some_host_pattern,
				Vec::new(),
				pattern,
				config_flags,
			);
		}

		if !path_pattern_str.starts_with('/') {
			panic!("path pattern must start with a slash or must be a root '/'")
		}

		let mut route_segments = RouteSegments::new(path_pattern_str);

		let mut prefix_path_pattern = Vec::new();

		let resource_pattern = loop {
			let (route_segment, _) = route_segments
				.next()
				.expect("local checks should validate that the next segment exists");

			let pattern = Pattern::parse(route_segment);

			if route_segments.has_remaining_segments() {
				prefix_path_pattern.push(pattern);

				continue;
			}

			break pattern;
		};

		Self::with_uri_pattern_and_config_flags(
			some_host_pattern,
			prefix_path_pattern,
			resource_pattern,
			config_flags,
		)
	}

	fn with_uri_pattern_and_config_flags(
		some_host_pattern: Option<Pattern>,
		prefix_path_pattern: Vec<Pattern>,
		resource_pattern: Pattern,
		config_flags: ConfigFlags,
	) -> Resource {
		Resource {
			pattern: resource_pattern,
			prefix_segment_patterns: prefix_path_pattern,
			some_host_pattern,
			static_resources: Vec::new(),
			regex_resources: Vec::new(),
			some_wildcard_resource: None,
			method_handlers: MethodHandlers::new(),
			some_mistargeted_request_handler: None,
			context_properties: ContextProperties::default(),
			extensions: Extensions::new(),
			middleware: Vec::new(),
			config_flags,
		}
	}

	#[inline(always)]
	pub(crate) fn with_pattern(pattern: Pattern) -> Resource {
		Self::with_uri_pattern_and_config_flags(None, Vec::new(), pattern, ConfigFlags::default())
	}

	// -------------------------

	/// Retuns true if the given host pattern is the one the resource was created with.
	///
	/// ```
	/// use argan::Resource;
	///
	/// let resource_2 = Resource::new("http://example.com/resource_1/resource_2");
	/// assert!(resource_2.host_is("http://example.com"));
	/// assert!(resource_2.host_is("example.com"));
	/// ```
	#[inline(always)]
	pub fn host_is<P: AsRef<str>>(&self, host_pattern: P) -> bool {
		let host_pattern_str = host_pattern.as_ref();

		let host_pattern_str = host_pattern_str
			.strip_prefix("https://")
			.or_else(|| host_pattern_str.strip_prefix("http://"))
			.unwrap_or(host_pattern_str);

		let host_pattern = Pattern::parse(host_pattern_str);

		self
			.some_host_pattern
			.as_ref()
			.is_some_and(|self_host_pattern| {
				dbg!(self_host_pattern, &host_pattern);

				self_host_pattern.compare(&host_pattern) == Similarity::Same
			})
	}

	/// Retuns true if the given pattern is the resource's pattern.
	///
	/// ```
	/// use argan::Resource;
	///
	/// let resource_2 = Resource::new("http://example.com/resource_1/resource_2");
	/// assert!(resource_2.is("/resource_2"));
	/// assert!(resource_2.is("resource_2"));
	/// ```
	#[inline(always)]
	pub fn is<P: AsRef<str>>(&self, pattern: P) -> bool {
		let pattern_str = pattern.as_ref();

		let pattern_str = if pattern_str != "/" {
			let pattern_str = pattern_str.strip_prefix('/').unwrap_or(pattern_str);

			pattern_str.strip_suffix('/').unwrap_or(pattern_str)
		} else {
			pattern_str
		};

		let pattern = Pattern::parse(pattern_str);

		self.pattern.compare(&pattern) == Similarity::Same
	}

	// -------------------------

	#[inline(always)]
	pub(crate) fn pattern_string(&self) -> String {
		self.pattern.to_string()
	}

	#[cfg(test)]
	pub(crate) fn static_resources(&self) -> &Vec<Resource> {
		&self.static_resources
	}

	#[cfg(test)]
	pub(crate) fn regex_resources(&self) -> &Vec<Resource> {
		&self.regex_resources
	}

	#[cfg(test)]
	pub(crate) fn wildcard_resources(&self) -> Option<&Resource> {
		self
			.some_wildcard_resource
			.as_ref()
			.map(|boxed_resource| boxed_resource.as_ref())
	}

	#[inline(always)]
	pub(crate) fn set_host_pattern(&mut self, host_pattern: Pattern) {
		if self.some_host_pattern.is_some() {
			panic!("resource already has a host pattern")
		}

		self.some_host_pattern = Some(host_pattern)
	}

	#[inline(always)]
	pub(crate) fn host_pattern_ref(&self) -> Option<&Pattern> {
		self.some_host_pattern.as_ref()
	}

	#[inline(always)]
	pub(crate) fn is_subtree_handler(&self) -> bool {
		self.config_flags.has(ConfigFlags::SUBTREE_HANDLER)
	}

	#[inline(always)]
	pub(crate) fn can_handle_request(&self) -> bool {
		self.method_handlers.count() > 0
	}

	#[inline(always)]
	pub(crate) fn has_some_effect(&self) -> bool {
		self.method_handlers.has_some_effect() || !self.middleware.is_empty()
	}

	#[cfg(test)]
	pub(crate) fn ends_with_slash(&self) -> bool {
		self.config_flags.has(ConfigFlags::ENDS_WITH_SLASH)
	}

	#[cfg(test)]
	pub(crate) fn drops_on_unmatching_slash(&self) -> bool {
		self
			.config_flags
			.has(ConfigFlags::DROPS_ON_UNMATCHING_SLASH)
	}

	// -------------------------

	/// Adds the given resources to the current resource's subtree.
	///
	/// ```
	/// use argan::Resource;
	///
	/// let mut resource_2 = Resource::new("/resource_1/resource_2");
	/// let resource_3_0 = Resource::new("/resource_3_0");
	/// let resource_4 = Resource::new("/resource_1/resource_2/resource_3_1/resource_4");
	///
	/// resource_2.add_subresource([resource_3_0, resource_4]);
	/// ```
	///
	/// # Panics
	///
	/// - if the resource being added is a root resource
	///
	/// ```should_panic
	/// use argan::Resource;
	///
	/// let mut parent = Resource::new("/parent");
	/// let root = Resource::new("/");
	///
	/// parent.add_subresource(root);
	/// ```
	///
	/// - if the current resource's URI doesn't match the host and/or prefix path segments
	/// of the given resource
	///
	/// ```should_panic
	/// use argan::Resource;
	///
	/// let mut resource_2 = Resource::new("/resource_1/resource_2");
	///
	/// // resource_3 is supposed to belong to a host `example.com`.
	/// let resource_3 = Resource::new("http://example.com/resource_1/resource_2/resource_3");
	///
	/// resource_2.add_subresource(resource_3);
	/// ```
	///
	/// - if the resource or one of its subresources has a duplicate in the current resources's
	/// subtree and both of them have some handler set or a middleware applied
	///
	/// ```should_panic
	/// use argan::Resource;
	/// use argan::handler::{_get, _post};
	///
	/// let mut resource_1 = Resource::new("/resource_1");
	/// let mut resource_3 = Resource::new("/resource_2/resource_3");
	/// resource_3.set_handler_for(_get.to(|| async {}));
	///
	/// resource_1.add_subresource(resource_3);
	///
	/// let mut resource_2 = Resource::new("/resource_2");
	/// let mut resource_3 = Resource::new("/resource_3");
	/// resource_3.set_handler_for(_post.to(|| async {}));
	///
	/// resource_2.add_subresource(resource_3);
	///
	/// // This doesn't try to merge the handler sets of the duplicate resources.
	/// resource_1.add_subresource(resource_2);
	/// ```
	pub fn add_subresource<R, const N: usize>(&mut self, new_resources: R)
	where
		R: IntoArray<Resource, N>,
	{
		let new_resources = new_resources.into_array();

		for new_resource in new_resources {
			if new_resource.is("/") {
				panic!("a root resource cannot be a subresource");
			}

			self.add_single_subresource(new_resource);
		}
	}

	fn add_single_subresource(&mut self, mut new_resource: Resource) {
		if !new_resource.prefix_segment_patterns.is_empty() {
			let some_host_pattern = new_resource.some_host_pattern.take();
			let mut prefix_segment_patterns =
				std::mem::take(&mut new_resource.prefix_segment_patterns).into_iter();

			self.check_uri_segments_are_the_same(some_host_pattern, &mut prefix_segment_patterns);

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
						// must also keep the host and prefix segment patterns of the existing resource.
						$new_resource.some_host_pattern = existing_resource.some_host_pattern.take();
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
					$new_resource.some_host_pattern = self.some_host_pattern.clone();
					$new_resource.prefix_segment_patterns = self.path_pattern();
					$resources.push($new_resource);
				}
			};
		}

		// -----

		match &new_resource.pattern {
			Pattern::Static(_) => add_resource!(self.static_resources, new_resource),
			#[cfg(feature = "regex")]
			Pattern::Regex(..) => add_resource!(self.regex_resources, new_resource),
			Pattern::Wildcard(_) => {
				// Explanation inside the above macro 'add_resource!' also applies here.
				if let Some(mut wildcard_resource) = self.some_wildcard_resource.take() {
					if wildcard_resource.pattern.compare(&new_resource.pattern) == Similarity::Same {
						if !new_resource.has_some_effect() {
							wildcard_resource.keep_subresources(new_resource);
						} else if !wildcard_resource.has_some_effect() {
							new_resource.some_host_pattern = wildcard_resource.some_host_pattern.take();
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
					new_resource
						.some_host_pattern
						.clone_from(&self.some_host_pattern);
					new_resource.prefix_segment_patterns = self.path_pattern();
					self.some_wildcard_resource = Some(Box::new(new_resource));
				}
			}
		}
	}

	fn check_uri_segments_are_the_same(
		&self,
		some_host_pattern: Option<Pattern>,
		prefix_segment_patterns: &mut impl Iterator<Item = Pattern>,
	) {
		if let Some(host_pattern) = some_host_pattern {
			let Some(self_host_pattern) = self.some_host_pattern.as_ref() else {
				panic!("resource is intended to belong to a host {}", host_pattern);
			};

			if self_host_pattern.compare(&host_pattern) != Similarity::Same {
				panic!("no host '{}' exists", host_pattern);
			}
		}

		if !self.is("/") {
			let self_path_segment_patterns = self
				.prefix_segment_patterns
				.iter()
				.chain(std::iter::once(&self.pattern));

			for self_path_segment_pattern in self_path_segment_patterns {
				let Some(prefix_segment_pattern) = prefix_segment_patterns.next() else {
					panic!("prefix path patterns must be the same with the path patterns of the parent")
				};

				if self_path_segment_pattern.compare(&prefix_segment_pattern) != Similarity::Same {
					panic!(
						"no segment '{}' exists among the prefix path segments of the resource '{}'",
						prefix_segment_pattern,
						self.pattern_string(),
					)
				}
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
				#[cfg(feature = "regex")]
				Pattern::Regex(_, _) => {
					let some_position = leaf_resource
						.regex_resources
						.iter()
						.position(|resource| resource.pattern.compare(pattern) == Similarity::Same);

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
			if let Some(capture_name) = current_resource.find_duplicate_capture_name_in_the_path(&pattern)
			{
				panic!("capture name '{}' is not unique in the path", capture_name)
			}

			current_resource.add_subresource(Resource::with_pattern(pattern.clone()));

			(current_resource, _) =
				current_resource.by_patterns_leaf_resource_mut(std::iter::once(pattern));
		}

		current_resource
	}

	// Checks the names of the new resource and its subresources for uniqueness.
	fn check_names_are_unique_in_the_path(&self, new_resource: &Resource) {
		if let Some(capture_name) = self.find_duplicate_capture_name_in_the_path(&new_resource.pattern)
		{
			panic!("capture name '{}' is not unique in the path", capture_name);
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

			if let Some(capture_name) = self.find_duplicate_capture_name_in_the_path(&resource.pattern) {
				panic!("capture name '{}' is not unique in the path", capture_name);
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
	pub(crate) fn keep_subresources(&mut self, mut other: Resource) {
		macro_rules! keep_other_resources {
			(mut $resources:expr, mut $other_resources:expr) => {
				if !$other_resources.is_empty() {
					if $resources.is_empty() {
						for other_resource in $other_resources.iter_mut() {
							other_resource.prefix_segment_patterns = self.path_pattern();
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
									other_resource.some_host_pattern = resource.some_host_pattern.take();
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
								other_resource.some_host_pattern = self.some_host_pattern.clone();
								other_resource.prefix_segment_patterns = self.path_pattern();
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
						other_wildcard_resource.some_host_pattern = wildcard_resource.some_host_pattern.take();
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
				other_wildcard_resource
					.some_host_pattern
					.clone_from(&self.some_host_pattern);
				other_wildcard_resource.prefix_segment_patterns = self.path_pattern();
				self.some_wildcard_resource = Some(other_wildcard_resource);
			}
		}
	}

	#[inline(always)]
	fn path_pattern(&self) -> Vec<Pattern> {
		let mut prefix_patterns = self.prefix_segment_patterns.clone();
		prefix_patterns.push(self.pattern.clone());

		prefix_patterns
	}

	/// Adds the given resources under the prefix path segments relative to the current resource.
	///
	/// ```
	/// use argan::Resource;
	///
	/// let mut resource_1 = Resource::new("/resource_1");
	/// let resource_3 = Resource::new("/resource_3");
	///
	/// resource_1.add_subresource_under("/resource_2", resource_3);
	/// ```
	///
	/// When a new resource has prefix URI components, it's better to use
	/// [add_subresource()](Self::add_subresource()). But the following also works:
	/// ```
	/// # use argan::Resource;
	/// # let mut resource_1 = Resource::new("/resource_1");
	/// let resource_4 = Resource::new("/resource_1/resource_2/resource_3/resource_4");
	///
	/// // Note that resource_1 may or may not have an existing subresource resource_2.
	/// resource_1.add_subresource_under("/resource_2", resource_4);
	/// ```
	///
	/// # Panics
	///
	/// - if the new resource's URI components don't match the current resource's URI components
	/// and/or the given releative path pattern, respectively
	///
	/// ```should_panic
	/// use argan::Resource;
	///
	/// let mut resource_1 = Resource::new("/resource_1");
	/// let resource_4 = Resource::new("/resource_1/resource_2/resource_3/resource_4");
	///
	/// resource_1.add_subresource_under("/some_resource", resource_4);
	/// ```
	///
	/// Other **panic** conditions are the same as [add_subresource()](Self::add_subresource())'s
	/// conditions.
	pub fn add_subresource_under<P, R, const N: usize>(
		&mut self,
		relative_path_pattern: P,
		new_resources: R,
	) where
		P: AsRef<str>,
		R: IntoArray<Resource, N>,
	{
		let relative_path_pattern = relative_path_pattern.as_ref();
		if relative_path_pattern.is_empty() {
			panic!("empty relative path");
		}

		let new_resources = new_resources.into_array();

		for new_resource in new_resources {
			if new_resource.is("/") {
				panic!("a root resource cannot be a subresource");
			}

			self.add_single_subresource_under(relative_path_pattern, new_resource);
		}
	}

	fn add_single_subresource_under(
		&mut self,
		relative_path_pattern: &str,
		mut new_resource: Resource,
	) {
		if !new_resource.prefix_segment_patterns.is_empty() {
			let some_host_pattern = new_resource.some_host_pattern.take();
			let mut prefix_segment_patterns =
				std::mem::take(&mut new_resource.prefix_segment_patterns).into_iter();

			// Prefix segments are absolute. They must be the same as the path segments of self.
			self.check_uri_segments_are_the_same(some_host_pattern, &mut prefix_segment_patterns);

			if relative_path_pattern.is_empty() {
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

			let prefix_route_segments = RouteSegments::new(relative_path_pattern);
			for (prefix_route_segment, _) in prefix_route_segments {
				let Some(prefix_segment_pattern) = prefix_segment_patterns.next() else {
					panic!(
						"new resource has fewer prefix path segments specified than where it's being added",
					)
				};

				// Keeps the complete prefix segment patterns to construct subresources later.
				let prefix_route_segment_pattern = Pattern::parse(prefix_route_segment);

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

		if relative_path_pattern.is_empty() {
			self.add_subresource(new_resource);
		} else {
			let subresource_to_be_parent = self.subresource_mut(relative_path_pattern);
			subresource_to_be_parent.add_subresource(new_resource);
		}
	}

	/// Returns the resource at the given relative path. If the resource doesn't exist, it
	/// will be created.
	///
	/// The path is relative to the resource the method is called on.
	///
	/// ```
	/// use argan::Resource;
	///
	/// let mut resource_1 = Resource::new("/resource_1");
	/// let resource_3 = resource_1.subresource_mut("/resource_2/resource_3");
	/// ```
	///
	/// # Panics
	/// - if the given path is empty
	/// - if the path contains only a slash `/` (root cannot be a subresource)
	/// - if the path doesn't start with a slash `/`
	/// - if the resource has some handler set or middleware applied, and the given
	///   configuration symbols don't match its configuration
	/// ```should_panic
	/// use argan::{Resource, handler::{_get, _post}};
	///
	/// let mut root = Resource::new("/");
	/// root.subresource_mut("/resource_1 !*").set_handler_for([
	///   _get.to(|| async {}),
	///   _post.to(|| async {}),
	/// ]);
	///
	/// // ...
	///
	/// let resource_1 = root.subresource_mut("/resource_1");
	/// ```
	///
	/// For configuration symbols, see the [`crate documentation`](crate);
	pub fn subresource_mut<P>(&mut self, relative_path: P) -> &mut Resource
	where
		P: AsRef<str>,
	{
		let relative_path = relative_path.as_ref();

		if relative_path.is_empty() {
			panic!("empty relative path")
		}

		if relative_path == "/" {
			panic!("relative path cannot be a root")
		}

		if !relative_path.starts_with('/') {
			panic!("'{}' relative path must start with '/'", relative_path)
		}

		let (config_flags, relative_path) = resource_config_from(relative_path);

		let segments = RouteSegments::new(relative_path);
		let (leaf_resource_in_the_path, segments) = self.leaf_resource_mut(segments);

		let subresource = leaf_resource_in_the_path.new_subresource_mut(segments);

		if !subresource.has_some_effect() {
			subresource.config_flags = config_flags;
		} else if subresource.config_flags != config_flags {
			panic!(
				"mismatching config symbols with the existing resource at '{}'",
				relative_path
			);
		}

		subresource
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
				#[cfg(feature = "regex")]
				Pattern::Regex(_, _) => {
					let some_position = leaf_resource
						.regex_resources
						.iter()
						.position(|resource| resource.pattern.compare(&pattern) == Similarity::Same);

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
							.expect(SCOPE_VALIDITY);
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
				#[cfg(feature = "regex")]
				Pattern::Regex(_, _) => {
					let some_position = leaf_resource
						.regex_resources
						.iter()
						.position(|resource| resource.pattern.compare(&pattern) == Similarity::Same);

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

	fn new_subresource_mut(&mut self, segments: RouteSegments) -> &mut Resource {
		let mut current_resource = self;
		let mut newly_created = false;
		let ends_with_slash = segments.ends_with_slash();

		for (segment, _) in segments {
			let pattern = Pattern::parse(segment);

			#[cfg(feature = "regex")]
			if let Pattern::Regex(_, _) = &pattern {
				if let Some(capture_name) =
					current_resource.find_duplicate_capture_name_in_the_path(&pattern)
				{
					panic!("capture name '{}' is not unique in the path", capture_name)
				}
			}

			if let Pattern::Wildcard(_) = &pattern {
				if let Some(capture_name) =
					current_resource.find_duplicate_capture_name_in_the_path(&pattern)
				{
					panic!("capture name '{}' is not unique in the path", capture_name)
				}
			}

			current_resource.add_subresource(Resource::with_pattern(pattern));

			(current_resource, _) = current_resource.leaf_resource_mut(RouteSegments::new(segment));
			newly_created = true;
		}

		if newly_created && ends_with_slash {
			current_resource
				.config_flags
				.add(ConfigFlags::ENDS_WITH_SLASH);
		}

		current_resource
	}

	fn find_duplicate_capture_name_in_the_path<'p>(&self, pattern: &'p Pattern) -> Option<&'p str> {
		match pattern {
			#[cfg(feature = "regex")]
			Pattern::Regex(capture_names, _) => {
				for prefix_pattern in self.prefix_segment_patterns.iter() {
					match prefix_pattern {
						Pattern::Regex(other_capture_names, _) => {
							let some_capture_name = capture_names
								.as_ref()
								.iter()
								.find(|(capture_name, _)| other_capture_names.has(capture_name.as_ref()));

							if let Some((capture_name, _)) = some_capture_name {
								return Some(capture_name.as_ref());
							}
						}
						Pattern::Wildcard(other_capture_name) => {
							let some_capture_name = capture_names
								.as_ref()
								.iter()
								.find(|(capture_name, _)| capture_name.as_ref() == other_capture_name.as_ref());

							if let Some((capture_name, _)) = some_capture_name {
								return Some(capture_name.as_ref());
							}
						}
						Pattern::Static(_) => {}
					}
				}

				None
			}
			Pattern::Wildcard(capture_name) => {
				for prefix_pattern in self.prefix_segment_patterns.iter() {
					match prefix_pattern {
						#[cfg(feature = "regex")]
						Pattern::Regex(other_capture_names, _) => {
							if other_capture_names
								.as_ref()
								.iter()
								.any(|(other_capture_name, _)| capture_name.as_ref() == other_capture_name.as_ref())
							{
								return Some(capture_name.as_ref());
							}
						}
						Pattern::Wildcard(other_capture_name) => {
							if capture_name.as_ref() == other_capture_name.as_ref() {
								return Some(capture_name.as_ref());
							}
						}
						Pattern::Static(_) => {}
					}
				}

				None
			}
			Pattern::Static(_) => None,
		}
	}

	// -------------------------

	/// Adds the given extension to the `Resource`. Added extensions are available to all the
	/// handlers of the `Resource` and to all the middleware that wrap these handlers via the
	/// [`Args`](crate::handler::Args) field [`NodeExtensions`](crate::common::NodeExtensions).
	///
	/// # Panics
	///
	/// - if an extension of the same type already exists
	pub fn add_extension<E: Clone + Send + Sync + 'static>(&mut self, extension: E) {
		if self.extensions.insert(extension).is_some() {
			panic!(
				"resource already has an extension of type '{}'",
				any::type_name::<E>()
			);
		}
	}

	// pub fn extension_ref<E: Clone + Send + Sync + 'static>(&self) -> &E {
	// 	self.extensions.get::<E>().expect(&format!(
	// 		"resource should have been provided with an extension of type '{}'",
	// 		any::type_name::<E>()
	// 	))
	// }

	/// Sets the method and mistargeted request handlers of the resource.
	///
	/// ```
	/// use argan::{Resource, handler::{_get, _method}};
	///
	/// let mut root = Resource::new("/");
	/// root.set_handler_for(_get.to(|| async {}));
	/// root.subresource_mut("/resource_1/resource_2").set_handler_for([
	///   _get.to(|| async {}),
	///   _method("LOCK").to(|| async {}),
	/// ]);
	/// ```
	pub fn set_handler_for<H, const N: usize>(&mut self, handler_kinds: H)
	where
		H: IntoArray<HandlerKind, N>,
	{
		let handler_kinds = handler_kinds.into_array();
		for handler_kind in handler_kinds {
			use HandlerKind::*;

			match handler_kind {
				Method(method, handler) => self.method_handlers.set_handler(method, handler),
				WildcardMethod(some_handler) => self
					.method_handlers
					.set_wildcard_method_handler(some_handler),
				MistargetedRequest(handler) => self.some_mistargeted_request_handler = Some(handler),
			}
		}
	}

	/// Adds middleware to be applied on the resource's components, like request receiver,
	/// passer, and method and other kind of handlers.
	///
	/// Middlewares are applied when the resource is being converted into a service.
	///
	/// ```
	/// // use declarations
	/// # use std::future::{Future, ready};
	/// # use tower_http::compression::CompressionLayer;
	/// # use http::method::Method;
	/// # use argan::{
	/// #   handler::{Handler, Args, _get},
	/// #   middleware::{Layer, _method_handler, _mistargeted_request_handler},
	/// #   resource::Resource,
	/// #   request::RequestContext,
	/// #   response::{Response, IntoResponse, BoxedErrorResponse},
	/// #   common::BoxedFuture,
	/// # };
	///
	/// #[derive(Clone)]
	/// struct MiddlewareLayer;
	///
	/// impl<H> Layer<H> for MiddlewareLayer
	/// where
	///   H: Handler + Clone + Send + Sync,
	/// {
	///   type Handler = Middleware<H>;
	///
	///   fn wrap(&self, handler: H) -> Self::Handler {
	///     Middleware(handler)
	///   }
	/// }
	///
	/// #[derive(Clone)]
	/// struct Middleware<H>(H);
	///
	/// impl<B, H> Handler<B> for Middleware<H>
	/// where
	///   H: Handler + Clone + Send + Sync,
	/// {
	///   type Response = Response;
	///   type Error = BoxedErrorResponse;
	///   type Future = BoxedFuture<Result<Self::Response, Self::Error>>;
	///
	///   fn handle(&self, request: RequestContext<B>, args: Args<'_, ()>) -> Self::Future {
	///     Box::pin(ready(Ok("Hello from Middleware!".into_response())))
	///   }
	/// }
	///
	/// let mut resource = Resource::new("/resource");
	///
	/// resource.add_layer_to([
	///   _mistargeted_request_handler(MiddlewareLayer),
	///   _method_handler(Method::GET, CompressionLayer::new()),
	/// ]);
	///
	/// resource.set_handler_for(_get.to(|| async {}));
	/// ```
	pub fn add_layer_to<L, const N: usize>(&mut self, layer_targets: L)
	where
		L: IntoArray<LayerTarget<Self>, N>,
	{
		self.middleware.extend(layer_targets.into_array());
	}

	/// Configures the resource with the given options.
	///
	/// ```
	/// use argan::{Resource, common::config::_with_request_extensions_modifier};
	///
	/// let mut resource = Resource::new("/resource");
	/// resource.configure(_with_request_extensions_modifier(|extensions| { /* ... */ }));
	/// ```
	pub fn configure<C, const N: usize>(&mut self, config_options: C)
	where
		C: IntoArray<ConfigOption<Self>, N>,
	{
		let config_options = config_options.into_array();

		for config_option in config_options {
			use ConfigOption::*;

			match config_option {
				#[cfg(any(feature = "private-cookies", feature = "signed-cookies"))]
				CookieKey(cookie_key) => self.context_properties.set_cookie_key(cookie_key),
				RequestExtensionsModifier(request_extensions_modifier_layer) => {
					let request_receiver_layer_target = _request_receiver(request_extensions_modifier_layer);

					self.middleware.insert(0, request_receiver_layer_target);
				}
				_ => {}
			}
		}
	}

	// -------------------------

	/// Calls the given function for each subresource with a mutable reference to the parameter.
	///
	/// If the function returns `Iteration::Skip` for any resource it's called for, that
	/// resource's subresources will be skipped. If the function retuns `Iteration::Stop` or
	/// all the subresources have been processed, the parameter is returned in its final state.
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
				Iteration::Skip => continue,
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

	/// Converts the `Resource` into a service.
	///
	/// This method ignores the parent resources. Thus, it should be called on the first
	/// resource in the resource tree.
	pub fn into_service(self) -> ResourceService {
		let Resource {
			pattern,
			prefix_segment_patterns: __prefix_segment_patterns,
			some_host_pattern: __some_host_pattern,
			static_resources,
			regex_resources,
			some_wildcard_resource,
			method_handlers,
			some_mistargeted_request_handler,
			context_properties: context,
			mut extensions,
			mut middleware,
			config_flags,
		} = self;

		// ----------

		let some_static_resources = if static_resources.is_empty() {
			None
		} else {
			Some(
				static_resources
					.into_iter()
					.map(Resource::into_service)
					.collect(),
			)
		};

		let some_regex_resources = if regex_resources.is_empty() {
			None
		} else {
			Some(
				regex_resources
					.into_iter()
					.map(Resource::into_service)
					.collect(),
			)
		};

		let some_wildcard_resource =
			some_wildcard_resource.map(|resource| Arc::new(resource.into_service()));

		// ----------

		let MethodHandlers {
			method_handlers_list,
			wildcard_method_handler,
			implemented_methods,
		} = method_handlers;

		if !implemented_methods.is_empty() {
			extensions.insert(ImplementedMethods::new(implemented_methods));
		}

		// -------------------------
		// MistargetedRequestHandller

		let some_mistargeted_request_handler =
			wrap_mistargeted_request_handler(some_mistargeted_request_handler, &mut middleware)
				.map(Into::into);

		// -------------------------
		// RequestHandler

		let some_request_handler =
			if method_handlers_list.is_empty() && !wildcard_method_handler.is_custom() {
				None
			} else {
				match RequestHandler::new(
					method_handlers_list,
					wildcard_method_handler,
					&mut middleware,
					some_mistargeted_request_handler.clone(),
				) {
					Ok(request_handler) => Some(Arc::new(request_handler)),
					Err(method) => panic!(
						"{} resource has no {} method handler to wrap",
						pattern, method
					),
				}
			};

		// -------------------------
		// RequestPasser

		let some_request_passer = if some_static_resources.is_some()
			|| some_regex_resources.is_some()
			|| some_wildcard_resource.is_some()
		{
			Some(RequestPasser::new(
				some_static_resources,
				some_regex_resources,
				some_wildcard_resource,
				some_mistargeted_request_handler.clone(),
				&mut middleware,
			))
		} else {
			None
		};

		// -------------------------
		// RequestReceiver

		let request_receiver = RequestReceiver::new(
			some_request_passer,
			some_request_handler,
			some_mistargeted_request_handler.clone(),
			config_flags.clone(),
			middleware,
		);

		// -------------------------
		// ResourceService

		ResourceService::new(
			pattern,
			context,
			extensions,
			request_receiver,
			some_mistargeted_request_handler,
		)
	}

	/// Converts the `Resource` into a service that uses `Arc` internally.
	///
	/// This method ignores the parent resources. Thus, it should be called on the first
	/// resource in the resource tree.
	#[inline(always)]
	pub fn into_arc_service(self) -> ArcResourceService {
		ArcResourceService::from(self.into_service())
	}

	/// Converts the `Resource` into a service with a leaked `&'static`.
	///
	/// This method ignores the parent resources. Thus, it should be called on the first
	/// resource in the resource tree.
	#[inline(always)]
	pub fn into_leaked_service(self) -> LeakedResourceService {
		LeakedResourceService::from(self.into_service())
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
				host_pattern exists: {},
				static_resources count: {},
				regex_resources count: {},
				wildcard_resource exists: {},
				middleware count: {},
				method_handlers: {{ count: {}, wildcard_method_handler_exists: {} }},
				mistargeted_request_handler exists: {},
				extensions count: {},
				config_flags: [{}],
			}}",
			&self.pattern,
			patterns_to_route(&self.prefix_segment_patterns),
			self.some_host_pattern.is_some(),
			self.static_resources.len(),
			self.regex_resources.len(),
			self.some_wildcard_resource.is_some(),
			self.middleware.len(),
			self.method_handlers.count(),
			self.method_handlers.has_custom_wildcard_method_handler(),
			self.some_mistargeted_request_handler.is_some(),
			self.extensions.len(),
			self.config_flags,
		)
	}
}

impl IntoArray<Resource, 1> for Resource {
	fn into_array(self) -> [Resource; 1] {
		[self]
	}
}

// --------------------------------------------------

/// Returned by a function given to the [`Resource::for_each_subresource()`] to control
/// the iteration on subresources.
pub enum Iteration {
	/// Iteration goes on normally.
	Continue,
	/// Current resource's subresources are skipped.
	Skip,
	/// Iteration stops.
	Stop,
}

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

#[cfg(all(test, feature = "full"))]
mod test {
	use crate::{
		common::{config::_with_request_extensions_modifier, route_to_patterns},
		handler::{DummyHandler, _get, _post, _put},
	};

	use super::*;

	// --------------------------------------------------------------------------------

	#[test]
	fn resource_new() {
		enum PatternType {
			Static,
			Regex,
			Wildcard,
		}

		let cases = [
			(
				"http:///st_0_0",
				ConfigFlags::REDIRECTS_ON_UNMATCHING_SLASH,
				PatternType::Static,
			),
			(
				"https:///st_0_0/{rx_1_0:p} ?",
				ConfigFlags::NONE,
				PatternType::Regex,
			),
			(
				"http:///st_0_0 !",
				ConfigFlags::DROPS_ON_UNMATCHING_SLASH,
				PatternType::Static,
			),
			(
				"/{wl_0_0)}/ ?",
				ConfigFlags::ENDS_WITH_SLASH,
				PatternType::Wildcard,
			),
			(
				"http://{sub}.example.com/st_0_0/ !",
				ConfigFlags::ENDS_WITH_SLASH | ConfigFlags::DROPS_ON_UNMATCHING_SLASH,
				PatternType::Static,
			),
			(
				"https://example.com/st_0_0/{wl_1_0} ?*",
				ConfigFlags::SUBTREE_HANDLER,
				PatternType::Wildcard,
			),
			(
				"/{rx_0_0:p}-abc !*",
				ConfigFlags::DROPS_ON_UNMATCHING_SLASH | ConfigFlags::SUBTREE_HANDLER,
				PatternType::Regex,
			),
			(
				"/{wl_0_0)}/{rx_1_0:p}/ ?*",
				ConfigFlags::ENDS_WITH_SLASH | ConfigFlags::SUBTREE_HANDLER,
				PatternType::Regex,
			),
			(
				"/st_0_0/st_1_0/ !*",
				ConfigFlags::ENDS_WITH_SLASH
					| ConfigFlags::DROPS_ON_UNMATCHING_SLASH
					| ConfigFlags::SUBTREE_HANDLER,
				PatternType::Static,
			),
			(
				"http://example.com/{wl_0_0} *",
				ConfigFlags::REDIRECTS_ON_UNMATCHING_SLASH | ConfigFlags::SUBTREE_HANDLER,
				PatternType::Wildcard,
			),
			(
				"http:///{wl_0_0}/st_1_0/ *",
				ConfigFlags::ENDS_WITH_SLASH
					| ConfigFlags::REDIRECTS_ON_UNMATCHING_SLASH
					| ConfigFlags::SUBTREE_HANDLER,
				PatternType::Static,
			),
			(
				"http://www.example.com/ *",
				ConfigFlags::SUBTREE_HANDLER,
				PatternType::Static,
			),
		];

		for (uri_pattern, config_flags, pattern_type) in cases {
			let resource = Resource::new(uri_pattern);

			match pattern_type {
				PatternType::Static => assert!(resource.pattern.is_static()),
				PatternType::Regex => assert!(resource.pattern.is_regex()),
				PatternType::Wildcard => assert!(resource.pattern.is_wildcard()),
			}

			assert_eq!(resource.config_flags, config_flags);
		}
	}

	#[test]
	#[should_panic(expected = "empty URI")]
	fn resource_new_with_empty_pattern() {
		Resource::new("");
	}

	#[test]
	#[should_panic(expected = "must start with a slash")]
	fn resource_new_with_invalid_path_pattern() {
		Resource::new("products/{category}");
	}

	#[test]
	fn resource_add_subresource() {
		//	/st_0_0	->	/{wl_1_0}	->	/{rx_2_0:p0}	->	/st_3_0
		//											|									|	->	/{rx_3_1:p0}	->	/{wl_4_0}
		//											|																		|	->	/st_4_1
		//											|																		|	->	/st_4_2
		//											|
		//											|	->	/st_2_1	->	/{wl_3_0}	->	/{rx_4_0:p0}
		//																								|	->	/{rx_4_1:p1}

		let parent_route = "/st_0_0/{wl_1_0}".to_string();
		let mut parent = Resource::new(&parent_route);

		let cases = [
			"/{rx_2_0:p0}/",
			"/{rx_2_0:p0}/st_3_0",
			"/{rx_2_0:p0}/{rx_3_1:p0}",
			"/{rx_2_0:p0}/{rx_3_1:p0}/{wl_4_0}",
			"/{rx_2_0:p0}/{rx_3_1:p0}/st_4_1",
			"/{rx_2_0:p0}/{rx_3_1:p0}/st_4_2",
			"/st_2_1",
			"/st_2_1/{wl_3_0}/{rx_4_0:p0}",
			"/st_2_1/{wl_3_0}/{rx_4_1:p1}/",
			"/st_2_1/{wl_3_0}",
		];

		for case in cases {
			let resource_path = parent_route.clone() + case;
			let resource = Resource::new(&resource_path);

			parent.add_subresource(resource);

			let (resource, _) = parent.by_patterns_leaf_resource_mut(route_to_patterns(case).into_iter());
			let prefix_patterns = route_to_patterns(&resource_path);
			resource.check_uri_segments_are_the_same(None, &mut prefix_patterns.into_iter());
		}

		{
			// Existing resources in the tree.

			let (rx_2_0, _) = parent.leaf_resource_mut(RouteSegments::new("/{rx_2_0:p0}"));
			rx_2_0.set_handler_for(_post.to(DummyHandler));
			rx_2_0
				.subresource_mut("/{rx_3_1:p0}/{wl_4_0}")
				.set_handler_for(_get.to(DummyHandler));

			let (st_4_2, _) = rx_2_0.leaf_resource_mut(RouteSegments::new("/{rx_3_1:p0}/st_4_2"));

			// New child.
			st_4_2.new_subresource_mut(RouteSegments::new("/st_5_0"));
		}

		{
			// New resources.

			let mut new_rx_2_0 = Resource::new("/{rx_2_0:p0}");

			let mut new_rx_3_1 = Resource::new("/{rx_3_1:p0}");
			// Must replace the existing resource.
			new_rx_3_1.set_handler_for([_get.to(DummyHandler), _post.to(DummyHandler)]);

			new_rx_3_1
				.subresource_mut("/st_4_1")
				// Must replace the existing resource.
				.set_handler_for(_post.to(DummyHandler));

			new_rx_3_1.new_subresource_mut(RouteSegments::new("/{rx_4_3:p0}"));
			new_rx_3_1.new_subresource_mut(RouteSegments::new("/st_4_4"));

			new_rx_2_0.add_subresource(new_rx_3_1);

			// Resources with handlers must replace existing resources with the same pattern.
			// Other resources must be kept as is.
			parent.add_subresource(new_rx_2_0);

			let (rx_2_0, _) = parent.leaf_resource(RouteSegments::new("/{rx_2_0:p0}"));
			assert_eq!(rx_2_0.static_resources.len(), 1);
			assert_eq!(rx_2_0.regex_resources.len(), 1);
			assert_eq!(rx_2_0.method_handlers.count(), 1);

			let (rx_3_1, _) = parent.leaf_resource(RouteSegments::new("/{rx_2_0:p0}/{rx_3_1:p0}"));
			assert_eq!(rx_3_1.static_resources.len(), 3);
			assert_eq!(rx_3_1.regex_resources.len(), 1);
			assert!(rx_3_1.some_wildcard_resource.is_some());
			assert_eq!(rx_3_1.method_handlers.count(), 2);

			let (wl_4_0, _) = rx_3_1.leaf_resource(RouteSegments::new("/{wl_4_0}"));
			assert_eq!(wl_4_0.method_handlers.count(), 1);

			let (st_4_2, _) = rx_3_1.leaf_resource(RouteSegments::new("/st_4_2"));
			assert_eq!(st_4_2.static_resources.len(), 1);

			let (st_5_0, _) = st_4_2.leaf_resource(RouteSegments::new("/st_5_0"));
			st_5_0.check_uri_segments_are_the_same(
				None,
				&mut route_to_patterns("/st_0_0/{wl_1_0}/{rx_2_0:p0}/{rx_3_1:p0}/st_4_2/st_5_0")
					.into_iter(),
			);

			let (st_4_1, _) = rx_3_1.leaf_resource(RouteSegments::new("/st_4_1"));
			assert_eq!(st_4_1.method_handlers.count(), 1);
		}

		{
			let wl_3_0_path = "/st_0_0/{wl_1_0}/st_2_1/{wl_3_0}";
			let wl_3_0_route = "/st_2_1/{wl_3_0}";

			let mut new_wl_3_0 = Resource::new(wl_3_0_path);
			new_wl_3_0.by_patterns_new_subresource_mut(std::iter::once(Pattern::parse("st_4_1")));
			new_wl_3_0.set_handler_for(_get.to(DummyHandler));

			parent.add_subresource(new_wl_3_0);
			let (wl_3_0, _) = parent.leaf_resource_mut(RouteSegments::new(wl_3_0_route));
			wl_3_0.check_uri_segments_are_the_same(None, &mut route_to_patterns(wl_3_0_path).into_iter());
			assert_eq!(wl_3_0.static_resources.len(), 1);
			assert_eq!(wl_3_0.regex_resources.len(), 2);
			assert_eq!(wl_3_0.method_handlers.count(), 1);
		}
	}

	#[test]
	fn resource_check_uri_segments_are_the_same() {
		let path_patterns = [
			"/news",
			"/news/{area:local|worldwide}",
			"/products/",
			"/products/{category}",
			"/products/{category}/{page:\\d+}/",
			"/{forecast_days:5|10}-days-forecast/{city}",
		];

		for path_pattern in path_patterns {
			let resource = Resource::new(path_pattern);
			let path_pattern = route_to_patterns(path_pattern);

			resource.check_uri_segments_are_the_same(None, &mut path_pattern.into_iter());
		}

		let resource_with_host = Resource::new("http://example.com/products/item");
		let host_pattern = Pattern::parse("example.com");
		let path_pattern = route_to_patterns("/products/item");

		resource_with_host
			.check_uri_segments_are_the_same(Some(host_pattern), &mut path_pattern.into_iter());
	}

	#[test]
	#[should_panic(expected = "resource is intended to belong to a host")]
	fn resource_check_uri_segments_are_the_same_panic1() {
		let resource = Resource::new("/news/{area:local|worldwide}");
		dbg!(&resource);
		let mut segment_patterns = vec![
			Pattern::parse("news"),
			Pattern::parse("{area:local|worldwide}"),
		]
		.into_iter();

		resource
			.check_uri_segments_are_the_same(Some(Pattern::parse("example.com")), &mut segment_patterns);
	}

	#[test]
	#[should_panic(expected = "no host")]
	fn resource_check_uri_segments_are_the_same_panic2() {
		let resource = Resource::new("http://example1.com/news/{area:local|worldwide}");
		dbg!(&resource);
		let mut segment_patterns = vec![
			Pattern::parse("news"),
			Pattern::parse("{area:local|worldwide}"),
		]
		.into_iter();

		resource
			.check_uri_segments_are_the_same(Some(Pattern::parse("example2.com")), &mut segment_patterns);
	}

	#[test]
	#[should_panic(expected = "patterns must be the same")]
	fn resource_check_uri_segments_are_the_same_panic3() {
		let resource = Resource::new("/news/{area:local|worldwide}");
		let mut segment_patterns = vec![Pattern::parse("news")].into_iter();

		resource.check_uri_segments_are_the_same(None, &mut segment_patterns);
	}

	#[test]
	#[should_panic(expected = "no segment")]
	fn resource_check_uri_segments_are_the_same_panic4() {
		let resource = Resource::new("/news/{area:local|worldwide}");
		let mut segment_patterns = vec![Pattern::parse("news"), Pattern::parse("{area}")].into_iter();

		resource.check_uri_segments_are_the_same(None, &mut segment_patterns);
	}

	#[test]
	#[should_panic(expected = "is not unique in the path")]
	fn resource_check_names_are_unique_in_the_path1() {
		let mut parent = Resource::new("/st_0_0/{rx_1_0:p1}/{wl_2_0}");
		let resource = Resource::new("/{rx_1_0:p2}");

		parent.add_subresource(resource);
	}

	#[test]
	#[should_panic(expected = "is not unique in the path")]
	fn resource_check_names_are_unique_in_the_path2() {
		let mut parent = Resource::new("/st_0_0/{rx_1_0:p1}/{wl_2_0}/st_3_0");
		let mut child = Resource::new("/st_4_0");
		let grandchild = Resource::new("/{rx_1_0:p2}");
		child.add_subresource(grandchild);

		parent.add_subresource(child);
	}

	#[test]
	fn resource_add_subresource_under() {
		//	/st_0_0	->	/{wl_1_0}	->	/{rx_2_0:p0}	->	/st_3_0
		//											|									|	->	/{rx_3_1:p0}	->	/{wl_4_0}
		//											|																		|	->	/st_4_1
		//											|																		|	->	/st_4_2
		//											|
		//											|	->	/st_2_1	->	/{wl_3_0}	->	/{rx_4_0:p0}
		//																	|							|	->	/{rx_4_1:p1}
		//																	|
		//																	|	->	/{rx_3_1:p0}

		let parent_route = "/st_0_0/{wl_1_0}".to_string();
		let mut parent = Resource::new(parent_route);

		struct Case<'a> {
			full_path: &'a str,
			prefix_route_from_parent: &'a str,
			resource_pattern: &'a str,
			route_from_parent: &'a str,
			resource_has_handler: bool,
		}

		let cases = [
			Case {
				full_path: "/st_0_0/{wl_1_0}/{rx_2_0:p0}/st_3_0",
				prefix_route_from_parent: "/{rx_2_0:p0}",
				resource_pattern: "st_3_0",
				route_from_parent: "/{rx_2_0:p0}/st_3_0",
				resource_has_handler: true,
			},
			Case {
				full_path: "/st_0_0/{wl_1_0}/{rx_2_0:p0}/{rx_3_1:p0}/",
				prefix_route_from_parent: "/{rx_2_0:p0}",
				resource_pattern: "{rx_3_1:p0}",
				route_from_parent: "/{rx_2_0:p0}/{rx_3_1:p0}/",
				resource_has_handler: true,
			},
			Case {
				full_path: "/st_0_0/{wl_1_0}/{rx_2_0:p0}/{rx_3_1:p0}/{wl_4_0}",
				prefix_route_from_parent: "/{rx_2_0:p0}/{rx_3_1:p0}/",
				resource_pattern: "{wl_4_0}",
				route_from_parent: "/{rx_2_0:p0}/{rx_3_1:p0}/{wl_4_0}",
				resource_has_handler: false,
			},
			Case {
				full_path: "/st_0_0/{wl_1_0}/{rx_2_0:p0}/{rx_3_1:p0}/st_4_1",
				prefix_route_from_parent: "/{rx_2_0:p0}",
				resource_pattern: "st_4_1",
				route_from_parent: "/{rx_2_0:p0}/{rx_3_1:p0}/st_4_1",
				resource_has_handler: true,
			},
			Case {
				full_path: "/st_0_0/{wl_1_0}/{rx_2_0:p0}/{rx_3_1:p0}/st_4_2",
				prefix_route_from_parent: "/{rx_2_0:p0}/{rx_3_1:p0}/",
				resource_pattern: "st_4_2",
				route_from_parent: "/{rx_2_0:p0}/{rx_3_1:p0}/st_4_2",
				resource_has_handler: false,
			},
			Case {
				full_path: "/st_0_0/{wl_1_0}/st_2_1/{wl_3_0}/",
				prefix_route_from_parent: "/st_2_1",
				resource_pattern: "{wl_3_0}",
				route_from_parent: "/st_2_1/{wl_3_0}",
				resource_has_handler: false,
			},
			Case {
				full_path: "/st_0_0/{wl_1_0}/st_2_1/{wl_3_0}/{rx_4_0:p0}/",
				prefix_route_from_parent: "/st_2_1/{wl_3_0}",
				resource_pattern: "{rx_4_0:p0}",
				route_from_parent: "/st_2_1/{wl_3_0}/{rx_4_0:p0}/",
				resource_has_handler: true,
			},
			Case {
				full_path: "/st_0_0/{wl_1_0}/st_2_1/{wl_3_0}/{rx_4_1:p1}/",
				prefix_route_from_parent: "/st_2_1/{wl_3_0}/",
				resource_pattern: "{rx_4_1:p1}",
				route_from_parent: "/st_2_1/{wl_3_0}/{rx_4_1:p1}",
				resource_has_handler: false,
			},
			Case {
				full_path: "/st_0_0/{wl_1_0}/st_2_1/{wl_3_0}/{rx_4_1:p1}/st_5_0",
				prefix_route_from_parent: "/st_2_1/{wl_3_0}/{rx_4_1:p1}",
				resource_pattern: "st_5_0",
				route_from_parent: "/st_2_1/{wl_3_0}/{rx_4_1:p1}/st_5_0",
				resource_has_handler: false,
			},
			Case {
				full_path: "/st_0_0/{wl_1_0}/st_2_1/{rx_3_1:p0}",
				prefix_route_from_parent: "/st_2_1",
				resource_pattern: "{rx_3_1:p0}",
				route_from_parent: "/st_2_1/{rx_3_1:p0}",
				resource_has_handler: false,
			},
		];

		for case in cases.iter() {
			dbg!(case.resource_pattern);

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
				resource.set_handler_for(_get.to(DummyHandler));
			}

			resource
				.check_uri_segments_are_the_same(None, &mut route_to_patterns(case.full_path).into_iter());
		}

		dbg!();

		{
			// Existing rx_3_1 has a handler. The new_ex_3_1 should not replace it.
			let mut new_rx_3_1 = Resource::new("/{rx_3_1:p0}");
			new_rx_3_1
				.subresource_mut("/{wl_4_0}")
				// Existing wl_4_0 doesn't have a handler. It should be replaced with the new one.
				.set_handler_for(_post.to(DummyHandler));

			// Existing st_4_1 has a handler. The new_st_4_1 should not replace it.
			let new_st_4_1 = Resource::new("/st_4_1");
			new_rx_3_1.add_subresource(new_st_4_1);
			new_rx_3_1.subresource_mut("/{rx_4_3:p0}");

			parent.add_subresource_under(cases[1].prefix_route_from_parent, new_rx_3_1);

			let (rx_3_1, _) = parent.leaf_resource(RouteSegments::new(cases[1].route_from_parent));
			assert_eq!(rx_3_1.static_resources.len(), 2);
			assert_eq!(rx_3_1.regex_resources.len(), 1);
			assert!(rx_3_1.some_wildcard_resource.is_some());
			assert_eq!(rx_3_1.method_handlers.count(), 1);

			let (wl_4_0, _) = parent.leaf_resource(RouteSegments::new(cases[2].route_from_parent));
			assert_eq!(wl_4_0.method_handlers.count(), 1);

			let (st_4_1, _) = parent.leaf_resource(RouteSegments::new(cases[3].route_from_parent));
			assert_eq!(st_4_1.method_handlers.count(), 1);
		}

		{
			let mut root = Resource::new("/");
			root.add_subresource(parent);

			// Existing st_2_1 doesn't have a handler. It should be replaced with the new one.
			let mut new_st_2_1 = Resource::new("/st_0_0/{wl_1_0}/st_2_1");
			new_st_2_1.set_handler_for(_get.to(DummyHandler));

			let mut new_rx_4_1 = Resource::new("/{rx_4_1:p1}");
			new_rx_4_1
				// New subresource.
				.subresource_mut("/{wl_5_1}")
				.set_handler_for(_get.to(DummyHandler));
			new_st_2_1.add_subresource_under("/{wl_3_0}", new_rx_4_1);

			let mut new_rx_4_1 = Resource::new("/{rx_4_1:p1}/");
			new_rx_4_1.set_handler_for(_put.to(DummyHandler));
			// Existing rx_4_1 shouldn't have a handler. It should be replaced with the new one.
			new_st_2_1.add_subresource_under("/{wl_3_0}", new_rx_4_1);

			let rx_5_0 = Resource::new("/st_0_0/{wl_1_0}/st_2_1/{rx_3_1:p0}/st_4_0/{rx_5_0:p0}");
			new_st_2_1.add_subresource_under("/{rx_3_1:p0}", rx_5_0);

			root.add_subresource_under("/st_0_0/{wl_1_0}/", new_st_2_1);

			let (st_2_1, _) = root.leaf_resource(RouteSegments::new("/st_0_0/{wl_1_0}/st_2_1"));
			assert_eq!(st_2_1.static_resources.len(), 0);
			assert_eq!(st_2_1.regex_resources.len(), 1);
			assert!(st_2_1.some_wildcard_resource.is_some());
			assert_eq!(st_2_1.method_handlers.count(), 1);

			let (rx_4_1, _) = st_2_1.leaf_resource(RouteSegments::new("/{wl_3_0}/{rx_4_1:p1}"));
			assert!(rx_4_1.some_wildcard_resource.is_some());
			assert_eq!(rx_4_1.method_handlers.count(), 1);

			let (rx_3_1, _) = st_2_1.leaf_resource(RouteSegments::new("/{rx_3_1:p0}"));
			assert_eq!(rx_3_1.static_resources.len(), 1);
			assert_eq!(rx_3_1.method_handlers.count(), 0);

			let (st_4_0, _) = st_2_1.leaf_resource(RouteSegments::new("/{rx_3_1:p0}/st_4_0"));
			assert_eq!(st_4_0.regex_resources.len(), 1);
		}
	}

	#[test]
	fn resource_subresource_mut() {
		//	/st_0_0	->	/{wl_1_0}	->	/{rx_2_0:p0}	->	/st_3_0
		//																				|	->	/{rx_3_1:p0}	->	/{wl_4_0}
		//																													|	->	/st_4_1

		let mut parent = Resource::new("https://example.com/");
		parent
			.subresource_mut("/st_0_0")
			.set_handler_for(_get.to(DummyHandler));

		parent
			.subresource_mut("/st_0_0/{wl_1_0}/{rx_2_0:p0}/st_3_0")
			.set_handler_for(_get.to(DummyHandler));

		parent
			.subresource_mut("/st_0_0/{wl_1_0}/{rx_2_0:p0}/{rx_3_1:p0}/{wl_4_0}/")
			.set_handler_for(_get.to(DummyHandler));

		let wl_1_0 = parent.subresource_mut("/st_0_0/{wl_1_0}");
		assert_eq!(wl_1_0.method_handlers.count(), 0);

		let st_0_0 = parent.subresource_mut("/st_0_0");
		assert_eq!(st_0_0.method_handlers.count(), 1);

		// First time we're accessing the rx_3_1. It must be configured to end with a slash.
		let rx_3_1 = parent.subresource_mut("/st_0_0/{wl_1_0}/{rx_2_0:p0}/{rx_3_1:p0}/");
		assert_eq!(rx_3_1.method_handlers.count(), 0);
		assert!(rx_3_1.config_flags.has(ConfigFlags::ENDS_WITH_SLASH));

		let st_3_0 = parent.subresource_mut("/st_0_0/{wl_1_0}/{rx_2_0:p0}/st_3_0");
		assert_eq!(st_3_0.method_handlers.count(), 1);

		let st_4_1 = parent.subresource_mut("/st_0_0/{wl_1_0}/{rx_2_0:p0}/{rx_3_1:p0}/st_4_1");
		assert_eq!(st_4_1.method_handlers.count(), 0);

		let wl_4_0 = parent.subresource_mut("/st_0_0/{wl_1_0}/{rx_2_0:p0}/{rx_3_1:p0}/{wl_4_0}/");
		assert_eq!(wl_4_0.method_handlers.count(), 1);
		assert!(wl_4_0.config_flags.has(ConfigFlags::ENDS_WITH_SLASH));
	}

	#[test]
	#[should_panic(expected = "mismatching config symbols")]
	fn resource_subresource_mut_panic1() {
		//	/st_0_0	->	/{wl_1_0}	->	/{rx_2_0:p}	->	/st_3_0

		let mut parent = Resource::new("https://example.com/");
		parent
			.subresource_mut("/st_0_0")
			.set_handler_for(_get.to(DummyHandler));

		parent
			.subresource_mut("/st_0_0/{wl_1_0}/{rx_2_0:p}/st_3_0")
			.set_handler_for(_get.to(DummyHandler));

		let _st_3_0 = parent.subresource_mut("/st_0_0/{wl_1_0}/{rx_2_0:p}/st_3_0/");
	}

	#[test]
	#[should_panic(expected = "mismatching config symbols")]
	fn resource_subresource_mut_panic2() {
		//	/st_0_0	->	/{wl_1_0}	->	/{rx_2_0:p}	->	/st_3_0

		let mut parent = Resource::new("https://example.com/");
		parent
			.subresource_mut("/st_0_0")
			.set_handler_for(_get.to(DummyHandler));

		parent
			.subresource_mut("/st_0_0/{wl_1_0}/{rx_2_0:p}/st_3_0/")
			.set_handler_for(_get.to(DummyHandler));

		let _st_3_0 = parent.subresource_mut("/st_0_0/{wl_1_0}/{rx_2_0:p}/st_3_0");
	}

	#[test]
	fn resource_for_each_subresource() {
		let mut parent = Resource::new("/");
		parent.subresource_mut("/st_0_0");
		parent.subresource_mut("/{rx_0_0:p}/{wl_1_0}/");
		parent.subresource_mut("/{wl_0_0}/st_1_0");

		parent.for_each_subresource((), |_, resource| {
			resource.configure(_with_request_extensions_modifier(|_: &mut Extensions| {}));
			if resource.is("{rx_0_0:p}") {
				Iteration::Skip
			} else {
				Iteration::Continue
			}
		});

		assert_eq!(parent.subresource_mut("/st_0_0").middleware.len(), 1);
		assert_eq!(parent.subresource_mut("/{rx_0_0:p}").middleware.len(), 1);
		assert!(parent
			.subresource_mut("/{rx_0_0:p}/{wl_1_0}/")
			.middleware
			.is_empty(),);

		assert_eq!(parent.subresource_mut("/{wl_0_0}").middleware.len(), 1);
		assert_eq!(
			parent.subresource_mut("/{wl_0_0}/st_1_0").middleware.len(),
			1
		);
	}
}
