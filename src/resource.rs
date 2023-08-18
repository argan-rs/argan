use std::{
	any::{Any, TypeId},
	ops::Deref,
	sync::Arc,
};

use crate::{
	body::IncomingBody,
	middleware::{IntoResponseAdapter, Layer, ResponseFutureBoxer},
	response::IntoResponse,
};

use super::{
	handler::{request_handlers::*, *},
	pattern::{Pattern, Similarity},
	request::Request,
	response::Response,
	routing::{Method, RoutingState, StatusCode, UnusedRequest},
};

use super::utils::*;

// --------------------------------------------------

pub struct Resource {
	pattern: Pattern,
	prefix_path_patterns: Vec<Pattern>,

	static_resources: Vec<Resource>,
	regex_resources: Vec<Resource>,
	wildcard_resource: Option<Box<Resource>>,

	request_receiver: Option<BoxedHandler>,
	request_passer: Option<BoxedHandler>,
	request_handler: Option<BoxedHandler>,

	method_handlers: MethodHandlers,

	state: Vec<Arc<dyn Any + Send + Sync>>,

	// TODO: configs, state, redirect, parent
	is_subtree_handler: bool,
}

// -------------------------

impl Resource {
	pub fn new(path_pattern: &str) -> Resource {
		if path_pattern.is_empty() {
			panic!("empty path pattern")
		}

		if path_pattern == "/" {
			let pattern = Pattern::parse(path_pattern);

			return Resource::with_pattern(pattern);
		}

		if !path_pattern.starts_with('/') {
			panic!("path pattern must start with a slash or must be a root pattern '/'")
		}

		let mut route_segments = RouteSegments::new(path_pattern);

		let mut resource_pattern: Pattern;
		let mut prefix_path_patterns = Vec::new();

		let resource_pattern = loop {
			let route_segment = route_segments.next().unwrap();
			let pattern = Pattern::parse(route_segment.as_str());

			if route_segments.has_remaining_segments() {
				prefix_path_patterns.push(pattern);

				continue;
			}

			break pattern;
		};

		Self::with_path_pattern(prefix_path_patterns, resource_pattern)
	}

	#[inline]
	pub(crate) fn with_pattern_str(pattern: &str) -> Resource {
		let pattern = Pattern::parse(pattern);

		Self::with_pattern(pattern)
	}

	#[inline]
	pub(crate) fn with_pattern(pattern: Pattern) -> Resource {
		Self::with_path_pattern(Vec::new(), pattern)
	}

	#[inline]
	pub(crate) fn with_path_pattern(
		prefix_path_patterns: Vec<Pattern>,
		resource_pattern: Pattern,
	) -> Resource {
		if let Pattern::Regex(ref name, None) = resource_pattern {
			panic!("{} pattern has no regex segment", name.as_ref())
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
			state: Vec::new(),
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
		self.method_handlers.is_empty()
	}

	#[inline]
	fn has_some_effect(&self) -> bool {
		self.request_handler.is_some()
			|| self.request_passer.is_some()
			|| self.request_receiver.is_some()
	}

	// -------------------------

	pub fn add_sub_resource(&mut self, mut new_resource: Resource) {
		if !new_resource.prefix_path_patterns.is_empty() {
			let mut prefix_path_patterns =
				std::mem::take(&mut new_resource.prefix_path_patterns).into_iter();

			self.check_path_patterns_are_the_same(&mut prefix_path_patterns);

			if prefix_path_patterns.len() > 0 {
				let sub_resource_to_be_parent = self.by_patterns_sub_resource_mut(prefix_path_patterns);
				sub_resource_to_be_parent.add_sub_resource(new_resource);

				return;
			}
		};

		if self.path_has_the_same_name_with_some_resources(&new_resource) {
			panic!("some resources has duplicate names in the path");
		}

		// -----

		macro_rules! add_resource {
			($resources:expr, $new_resource:ident) => {
				if let Some(position) = $resources
					.iter_mut()
					.position(|resource| resource.pattern.compare(&$new_resource.pattern) == Similarity::Same)
				{
					// TODO: Provide default constructor.
					let dummy_resource = Resource::with_pattern_str("dummy");
					let mut existing_resource = std::mem::replace(&mut $resources[position], dummy_resource);

					if !$new_resource.has_some_effect() {
						existing_resource.keep_sub_resources($new_resource);
					} else if !existing_resource.has_some_effect() {
						$new_resource.prefix_path_patterns =
							std::mem::take(&mut existing_resource.prefix_path_patterns);
						$new_resource.keep_sub_resources(existing_resource);
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
			};
		}

		// -----

		match &new_resource.pattern {
			Pattern::Static(_) => add_resource!(self.static_resources, new_resource),
			Pattern::Regex(..) => add_resource!(self.regex_resources, new_resource),
			Pattern::Wildcard(_) => {
				if let Some(mut wildcard_resource) = self.wildcard_resource.take() {
					if wildcard_resource.pattern.compare(&new_resource.pattern) == Similarity::Same {
						if !new_resource.has_some_effect() {
							wildcard_resource.keep_sub_resources(new_resource);
						} else if !wildcard_resource.has_some_effect() {
							new_resource.prefix_path_patterns =
								std::mem::take(&mut wildcard_resource.prefix_path_patterns);
							new_resource.keep_sub_resources(*wildcard_resource);
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
	fn check_path_patterns_are_the_same(
		&self,
		prefix_path_patterns: &mut impl Iterator<Item = Pattern>,
	) {
		let self_path_patterns = self
			.prefix_path_patterns
			.iter()
			.chain(std::iter::once(&self.pattern));
		for self_path_segment_pattern in self_path_patterns {
			let Some(prefix_path_segment_pattern) = prefix_path_patterns.next() else {
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
	fn by_patterns_sub_resource_mut(
		&mut self,
		mut patterns: impl Iterator<Item = Pattern>,
	) -> &mut Resource {
		let mut leaf_resource_in_the_path = self.by_patterns_leaf_resource_mut(&mut patterns);
		leaf_resource_in_the_path.by_patterns_new_sub_resource_mut(patterns)
	}

	#[inline]
	fn by_patterns_leaf_resource_mut(
		&mut self,
		patterns: &mut impl Iterator<Item = Pattern>,
	) -> &mut Resource {
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

		leaf_resource
	}

	#[inline]
	fn by_patterns_new_sub_resource_mut(
		&mut self,
		patterns: impl Iterator<Item = Pattern>,
	) -> &mut Resource {
		let mut current_resource = self;

		for pattern in patterns {
			if let Some(name) = pattern.name() {
				if current_resource.path_has_the_same_name(name) {
					panic!("{} is not unique in the path", name)
				}

				let pattern_clone = pattern.clone();
				let new_sub_resource = Resource::with_pattern(pattern);
				current_resource.add_sub_resource(new_sub_resource);
				current_resource =
					current_resource.by_patterns_leaf_resource_mut(&mut std::iter::once(pattern_clone));
			}
		}

		current_resource
	}

	#[inline]
	fn path_patterns(&mut self) -> Vec<Pattern> {
		let mut prefix_patterns = self.prefix_path_patterns.clone();
		prefix_patterns.push(self.pattern.clone());

		prefix_patterns
	}

	#[inline]
	fn path_has_the_same_name_with_some_resources(&self, new_resource: &Resource) -> bool {
		let mut resources = if new_resource.name().is_some() {
			vec![new_resource]
		} else {
			Vec::new()
		};

		loop {
			let Some(resource) = resources.pop() else {
				return false;
			};

			// Regex and wildcard resources all must have a name.
			// If the following unwrap() panics then we have a bug in our resource initialization logic.
			let name = new_resource.name().unwrap();
			if self.path_has_the_same_name(name) {
				return true;
			}

			resources.extend(resource.regex_resources.iter());

			if let Some(wildcard_resource) = &resource.wildcard_resource {
				resources.push(wildcard_resource);
			}
		}
	}

	fn keep_sub_resources(&mut self, mut other: Resource) {
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
									resource.keep_sub_resources(other_resource);
								} else if !resource.has_some_effect() {
									other_resource.prefix_path_patterns = std::mem::take(&mut resource.prefix_path_patterns);
									other_resource.keep_sub_resources(resource);
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
						wildcard_resource.keep_sub_resources(*other_wildcard_resource);
					} else if !wildcard_resource.has_some_effect() {
						other_wildcard_resource.prefix_path_patterns =
							std::mem::take(&mut wildcard_resource.prefix_path_patterns);
						other_wildcard_resource.keep_sub_resources(*wildcard_resource);
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

	pub fn add_sub_resource_under(&mut self, prefix_route: &str, mut new_resource: Resource) {
		if !new_resource.prefix_path_patterns.is_empty() {
			let mut new_resource_prefix_path_patterns =
				std::mem::take(&mut new_resource.prefix_path_patterns).into_iter();

			self.check_path_patterns_are_the_same(&mut new_resource_prefix_path_patterns);

			if prefix_route.is_empty() {
				if new_resource_prefix_path_patterns.len() > 0 {
					let sub_resource_to_be_parent =
						self.by_patterns_sub_resource_mut(new_resource_prefix_path_patterns);
					sub_resource_to_be_parent.add_sub_resource(new_resource);
				} else {
					self.add_sub_resource(new_resource);
				}

				return;
			}

			let mut prefix_route_patterns = Vec::new();

			let prefix_route_segments = RouteSegments::new(prefix_route);
			for prefix_route_segment in prefix_route_segments {
				let Some(prefix_path_segment_pattern) = new_resource_prefix_path_patterns.next() else {
					panic!("prefix path patterns must be the same with the path patterns of the parent")
				};

				let prefix_route_segment_pattern = Pattern::parse(prefix_route_segment.as_str());
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
					panic!("prefix path patterns must be the same with the path patterns of the parent")
				}

				prefix_route_patterns.push(prefix_route_segment_pattern);
			}

			// TODO: Create a new method to get sub_resource from patterns.
			let mut sub_resource_to_be_parent =
				self.by_patterns_sub_resource_mut(prefix_route_patterns.into_iter());
			if new_resource_prefix_path_patterns.len() > 0 {
				sub_resource_to_be_parent =
					sub_resource_to_be_parent.by_patterns_sub_resource_mut(new_resource_prefix_path_patterns);
			}

			sub_resource_to_be_parent.add_sub_resource(new_resource);

			return;
		}

		if prefix_route.is_empty() {
			self.add_sub_resource(new_resource);
		} else {
			let sub_resource_to_be_parent = self.sub_resource_mut(prefix_route);
			sub_resource_to_be_parent.add_sub_resource(new_resource);
		}
	}

	pub fn sub_resource_mut(&mut self, route: &str) -> &mut Resource {
		if route.is_empty() {
			panic!("empty route")
		}

		if route == "/" {
			panic!("root cannot be a sub-resource")
		}

		if !route.starts_with('/') {
			panic!("route must start with '/'")
		}

		let mut segments = RouteSegments::new(route);
		let mut leaf_resource_in_the_path = self.leaf_resource_mut(&mut segments);

		leaf_resource_in_the_path.new_sub_resource_mut(&mut segments)
	}

	#[inline]
	fn leaf_resource(&self, patterns: &mut RouteSegments) -> &Resource {
		let mut existing_resource = self;

		for segment in patterns.by_ref() {
			let pattern = Pattern::parse(segment.as_str());

			match pattern {
				Pattern::Static(_) => {
					let some_position = existing_resource
						.static_resources
						.iter()
						.position(|resource| resource.pattern.compare(&pattern) == Similarity::Same);

					if let Some(position) = some_position {
						existing_resource = &existing_resource.static_resources[position];
					} else {
						patterns.revert_to_segment(segment);

						break;
					}
				}
				Pattern::Regex(ref name, ref some_regex) => {
					let some_position = existing_resource
						.regex_resources
						.iter()
						.position(|resource| {
							if some_regex.is_some() {
								resource.pattern.compare(&pattern) == Similarity::Same
							} else {
								// Unwrap safety: Regex resources must have a name.
								resource.name().unwrap() == name.as_ref()
							}
						});

					if let Some(position) = some_position {
						existing_resource = &existing_resource.regex_resources[position];
					} else {
						patterns.revert_to_segment(segment);

						break;
					}
				}
				Pattern::Wildcard(_) => {
					if existing_resource
						.wildcard_resource
						.as_ref()
						.is_some_and(|resource| resource.pattern.compare(&pattern) == Similarity::Same)
					{
						existing_resource = existing_resource.wildcard_resource.as_deref().unwrap();
					} else {
						patterns.revert_to_segment(segment);

						break;
					}
				}
			}
		}

		existing_resource
	}

	#[inline]
	fn leaf_resource_mut(&mut self, patterns: &mut RouteSegments) -> &mut Resource {
		let mut existing_resource = self;

		for segment in patterns.by_ref() {
			let pattern = Pattern::parse(segment.as_str());

			match pattern {
				Pattern::Static(_) => {
					let some_position = existing_resource
						.static_resources
						.iter()
						.position(|resource| resource.pattern.compare(&pattern) == Similarity::Same);

					if let Some(position) = some_position {
						existing_resource = &mut existing_resource.static_resources[position];
					} else {
						patterns.revert_to_segment(segment);

						break;
					}
				}
				Pattern::Regex(ref name, ref some_regex) => {
					let some_position = existing_resource
						.regex_resources
						.iter()
						.position(|resource| {
							if some_regex.is_some() {
								resource.pattern.compare(&pattern) == Similarity::Same
							} else {
								// Unwrap safety: Regex resources must have a name.
								resource.name().unwrap() == name.as_ref()
							}
						});

					if let Some(position) = some_position {
						existing_resource = &mut existing_resource.regex_resources[position];
					} else {
						patterns.revert_to_segment(segment);

						break;
					}
				}
				Pattern::Wildcard(_) => {
					if existing_resource
						.wildcard_resource
						.as_ref()
						.is_some_and(|resource| resource.pattern.compare(&pattern) == Similarity::Same)
					{
						existing_resource = existing_resource.wildcard_resource.as_deref_mut().unwrap();
					} else {
						patterns.revert_to_segment(segment);

						break;
					}
				}
			}
		}

		existing_resource
	}

	#[inline]
	fn new_sub_resource_mut(&mut self, segments: &mut RouteSegments) -> &mut Resource {
		let mut current_resource = self;

		for segment in segments {
			let pattern = Pattern::parse(segment.as_str());

			if let Some(name) = pattern.name() {
				if current_resource.path_has_the_same_name(name) {
					panic!("{} is not unique in the path", name)
				}

				let new_sub_resource = Resource::with_pattern(pattern);
				current_resource.add_sub_resource(new_sub_resource);
				current_resource = current_resource.sub_resource_mut(segment.as_str());
			}
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

	pub fn set_state<S>(&mut self, state: S)
	where
		S: Clone + Send + Sync + 'static,
	{
		let state_type_id = state.type_id();

		if self
			.state
			.iter()
			.any(|existing_state| existing_state.deref().type_id() == state_type_id)
		{
			panic!(
				"resource already has a state of type '{:?}'",
				TypeId::of::<S>()
			);
		}

		self.state.push(Arc::new(state));
	}

	pub fn state<S>(&self) -> Option<&S>
	where
		S: Clone + Send + Sync + 'static,
	{
		self
			.state
			.iter()
			.find_map(|state| state.downcast_ref::<S>())
	}

	// pub fn set_config(&mut self, config: Config) {
	// 	todo!()
	// }
	//
	// pub fn config(&self) -> Config {
	// 	todo!()
	// }

	pub fn set_handler<H>(
		&mut self,
		method: Method,
		handler: impl IntoHandler<IncomingBody, Handler = H>,
	) where
		H: Handler + Sync + 'static,
		H::Response: IntoResponse,
	{
		let ready_handler =
			ResponseFutureBoxer::wrap(IntoResponseAdapter::wrap(handler.into_handler()));
		self
			.method_handlers
			.set_handler(method, ready_handler.into_boxed_handler())
	}

	// pub fn wrap_request_receiver<H>(&mut self, layer: impl Layer<H>)
	// where
	// 	H: Handler<
	// 		Response = Response,
	// 		Error = BoxedError,
	// 		Future = BoxedFuture<Result<Response, BoxedError>>,
	// 	>,
	// {
	// 	todo!()
	// }
	//
	// pub fn wrap_request_passer<H>(&mut self, layer: impl Layer<H>)
	// where
	// 	H: Handler<
	// 		Response = Response,
	// 		Error = BoxedError,
	// 		Future = BoxedFuture<Result<Response, BoxedError>>,
	// 	>,
	// {
	// 	todo!()
	// }
	//
	// pub fn wrap_request_handler<H>(&mut self, layer: impl Layer<H>)
	// where
	// 	H: Handler<
	// 		Response = Response,
	// 		Error = BoxedError,
	// 		Future = BoxedFuture<Result<Response, BoxedError>>,
	// 	>,
	// {
	// 	todo!()
	// }

	pub fn wrap_method_handler<L, LayeredB>(&mut self, method: Method, layer: L)
	where
		L: Layer<AdaptiveHandler<LayeredB>, LayeredB>,
		<L>::Handler: Handler<IncomingBody> + Sync + 'static,
		<L::Handler as Handler<IncomingBody>>::Response: IntoResponse,
	{
		self.method_handlers.wrap_handler(method, layer);
	}

	// -------------------------

	pub fn set_sub_resource_state<S>(&mut self, route: &str, state: S)
	where
		S: Clone + Send + Sync + 'static,
	{
		let sub_resource = self.sub_resource_mut(route);
		sub_resource.set_state(state);
	}

	pub fn sub_resource_state<S>(&self, route: &str) -> Option<&S>
	where
		S: Clone + Send + Sync + 'static,
	{
		let mut route_segments = RouteSegments::new(route);
		let sub_resource = self.leaf_resource(&mut route_segments);

		sub_resource.state()
	}

	// pub fn set_sub_resource_config(&mut self, route: &str, config: Config) {
	// 	let sub_resource = self.sub_resource_mut(route);
	// 	sub_resource.set_config(config);
	// }
	//
	// pub fn sub_resource_config(&self, route: &str) -> Config {
	// 	let mut route_segments = RouteSegments::new(route);
	// 	let sub_resource = self.leaf_resource(&mut route_segments);
	//
	// 	sub_resource.config()
	// }

	pub fn set_sub_resource_handler<H>(
		&mut self,
		route: &str,
		method: Method,
		handler: impl IntoHandler<IncomingBody, Handler = H>,
	) where
		H: Handler + Sync + 'static,
		H::Response: IntoResponse,
	{
		let sub_resource = self.sub_resource_mut(route);
		sub_resource.set_handler(method, handler);
	}

	// pub fn wrap_sub_resource_request_receiver<H>(&mut self, route: &str, layer: impl Layer<H>)
	// where
	// 	H: Handler<
	// 		Response = Response,
	// 		Error = BoxedError,
	// 		Future = BoxedFuture<Result<Response, BoxedError>>,
	// 	>,
	// {
	// 	todo!()
	// }
	//
	// pub fn wrap_sub_resource_request_passer<H>(&mut self, route: &str, layer: impl Layer<H>)
	// where
	// 	H: Handler<
	// 		Response = Response,
	// 		Error = BoxedError,
	// 		Future = BoxedFuture<Result<Response, BoxedError>>,
	// 	>,
	// {
	// 	todo!()
	// }
	//
	// pub fn wrap_sub_resource_request_handler<H>(&mut self, route: &str, layer: impl Layer<H>)
	// where
	// 	H: Handler<
	// 		Response = Response,
	// 		Error = BoxedError,
	// 		Future = BoxedFuture<Result<Response, BoxedError>>,
	// 	>,
	// {
	// 	todo!()
	// }
	//
	// pub fn wrap_sub_resource_method_handler<H>(
	// 	&mut self,
	// 	route: &str,
	// 	method: Method,
	// 	layer: impl Layer<H>,
	// ) where
	// 	H: Handler<
	// 		Response = Response,
	// 		Error = BoxedError,
	// 		Future = BoxedFuture<Result<Response, BoxedError>>,
	// 	>,
	// {
	// 	todo!()
	// }

	// -------------------------

	pub fn set_sub_resources_state<S>(&mut self, state: S)
	where
		S: Clone + Send + Sync + 'static,
	{
		let mut sub_resources = Vec::new();
		sub_resources.extend(self.static_resources.iter_mut());
		sub_resources.extend(self.regex_resources.iter_mut());
		if let Some(resource) = self.wildcard_resource.as_deref_mut() {
			sub_resources.push(resource);
		}

		for i in 0.. {
			let Some(sub_resource) = sub_resources.pop() else {
				break;
			};

			sub_resource.set_state(state.clone());

			sub_resources.extend(sub_resource.static_resources.iter_mut());
			sub_resources.extend(sub_resource.regex_resources.iter_mut());
			if let Some(resource) = sub_resource.wildcard_resource.as_deref_mut() {
				sub_resources.push(resource);
			}
		}
	}

	// pub fn set_sub_resources_config(&mut self, config: Config) {
	// 	todo!()
	// }

	// pub fn wrap_sub_resources_request_receivers<H>(&mut self, layer: impl Layer<H>)
	// where
	// 	H: Handler<
	// 		Response = Response,
	// 		Error = BoxedError,
	// 		Future = BoxedFuture<Result<Response, BoxedError>>,
	// 	>,
	// {
	// 	todo!()
	// }
	//
	// pub fn wrap_sub_resources_request_passers<H>(&mut self, layer: impl Layer<H>)
	// where
	// 	H: Handler<
	// 		Response = Response,
	// 		Error = BoxedError,
	// 		Future = BoxedFuture<Result<Response, BoxedError>>,
	// 	>,
	// {
	// 	todo!()
	// }
	//
	// pub fn wrap_sub_resources_request_handlers<H>(&mut self, layer: impl Layer<H>)
	// where
	// 	H: Handler<
	// 		Response = Response,
	// 		Error = BoxedError,
	// 		Future = BoxedFuture<Result<Response, BoxedError>>,
	// 	>,
	// {
	// 	todo!()
	// }
	//
	// pub fn wrap_sub_resources_method_handlers<H>(&mut self, method: Method, layer: impl Layer<H>)
	// where
	// 	H: Handler<
	// 		Response = Response,
	// 		Error = BoxedError,
	// 		Future = BoxedFuture<Result<Response, BoxedError>>,
	// 	>,
	// {
	// 	todo!()
	// }
}

// --------------------------------------------------

fn request_receiver(mut request: Request) -> BoxedFuture<Response> {
	Box::pin(async move {
		let mut routing_state = request.extensions_mut().get_mut::<RoutingState>().unwrap();
		let current_resource = routing_state.current_resource.unwrap();

		if routing_state.path_segments.has_remaining_segments() {
			if current_resource.is_subtree_handler() {
				routing_state.subtree_handler_exists = true;
			}

			let mut response = match current_resource.request_passer.as_ref() {
				Some(request_passer) => request_passer.handle(request).await,
				None => request_passer(request).await,
			};

			if response.status() != StatusCode::NOT_FOUND
				|| !current_resource.is_subtree_handler()
				|| !current_resource.can_handle_request()
			{
				return response;
			}

			let Some(unused_request) = response.extensions_mut().remove::<UnusedRequest>() else {
				return response;
			};

			request = unused_request.into_request()
		}

		if let Some(request_handler) = current_resource.request_handler.as_ref() {
			return request_handler.handle(request).await;
		}

		if current_resource.method_handlers.is_empty() {
			return misdirected_request_handler(request).await;
		}

		current_resource.method_handlers.handle(request).await
	})
}

#[inline]
async fn request_passer(mut request: Request) -> Response {
	let routing_state = request.extensions_mut().get_mut::<RoutingState>().unwrap();
	let current_resource = routing_state.current_resource.unwrap();
	let next_path_segment = routing_state.path_segments.next().unwrap();

	let some_next_resource = 'some_next_resource: {
		if let Some(next_resource) = current_resource
			.static_resources
			.iter()
			.find(|resource| resource.pattern.is_match(next_path_segment.as_str()))
		{
			break 'some_next_resource Some(next_resource);
		}

		if let Some(next_resource) = current_resource
			.regex_resources
			.iter()
			.find(|resource| resource.pattern.is_match(next_path_segment.as_str()))
		{
			break 'some_next_resource Some(next_resource);
		}

		current_resource.wildcard_resource.as_deref()
	};

	if let Some(next_resource) = some_next_resource {
		routing_state.current_resource.replace(next_resource);

		let mut response = match next_resource.request_receiver.as_ref() {
			Some(request_receiver) => request_receiver.handle(request).await,
			None => request_receiver(request).await,
		};

		let Some(unused_request) = response.extensions_mut().get_mut::<UnusedRequest>() else {
			return response;
		};

		let req = unused_request.as_mut();

		let routing_state = req.extensions_mut().get_mut::<RoutingState>().unwrap();
		routing_state.current_resource.replace(current_resource);
		routing_state
			.path_segments
			.revert_to_segment(next_path_segment);

		return response;
	}

	misdirected_request_handler(request).await
}

fn request_handler(mut request: Request) -> BoxedFuture<Response> {
	let routing_state = request.extensions_mut().get_mut::<RoutingState>().unwrap();
	let current_resource = routing_state.current_resource.unwrap();

	current_resource.method_handlers.handle(request)
}
