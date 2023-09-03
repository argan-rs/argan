use std::{
	any::{Any, TypeId},
	sync::Arc,
};

use crate::{
	body::IncomingBody,
	handler::{
		request_handlers::MethodHandlers, wrap_arc_handler, AdaptiveHandler, ArcHandler, Handler,
		IntoArcHandler, IntoHandler,
	},
	middleware::{IntoResponseAdapter, Layer, ResponseFutureBoxer},
	pattern::{Pattern, Similarity},
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

	state: Vec<Box<dyn Any + Send + Sync>>,

	// TODO: configs, state, redirect, parent
	is_subtree_handler: bool,
}

// -------------------------

impl Resource {
	pub fn new(path_pattern: &str) -> Result<Resource, &'static str> {
		if path_pattern.is_empty() {
			panic!("empty path pattern")
		}

		if path_pattern == "/" {
			let pattern = Pattern::parse(path_pattern)?;

			return Resource::with_pattern(pattern);
		}

		if !path_pattern.starts_with('/') {
			return Err("path pattern must start with a slash or must be a root pattern '/'");
		}

		let mut route_segments = RouteSegments::new(path_pattern);

		let mut resource_pattern: Pattern;
		let mut prefix_segment_patterns = Vec::new();

		let resource_pattern = loop {
			let (route_segment, _) = route_segments.next().unwrap();
			let pattern = Pattern::parse(route_segment)?;

			if route_segments.has_remaining_segments() {
				prefix_segment_patterns.push(pattern);

				continue;
			}

			break pattern;
		};

		Self::with_prefix_path_patterns(prefix_segment_patterns, resource_pattern)
	}

	#[inline]
	pub(crate) fn with_pattern_str(pattern: &str) -> Result<Resource, &'static str /*TODO*/> {
		let pattern = Pattern::parse(pattern)?;

		Self::with_pattern(pattern)
	}

	#[inline]
	pub(crate) fn with_pattern(pattern: Pattern) -> Result<Resource, &'static str /*TODO*/> {
		Self::with_prefix_path_patterns(Vec::new(), pattern)
	}

	#[inline]
	pub(crate) fn with_prefix_path_patterns(
		prefix_path_patterns: Vec<Pattern>,
		resource_pattern: Pattern,
	) -> Result<Resource, &'static str /*TODO*/> {
		if let Pattern::Regex(ref name, None) = resource_pattern {
			panic!("{} pattern has no regex segment", name.as_ref())
		}

		Ok(Resource {
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
		})
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

			self.check_path_segments_are_the_same(&mut prefix_path_patterns);

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
				if let Some(position) = $resources.iter_mut().position(
					|resource| resource.pattern.compare(&$new_resource.pattern) == Similarity::Same
				) {
					let dummy_resource = Resource::with_pattern_str("dummy").unwrap(); // TODO: Provide default constructor.
					let mut existing_resource = std::mem::replace(&mut $resources[position], dummy_resource);

					if !$new_resource.has_some_effect() {
						existing_resource.keep_sub_resources($new_resource);
					} else if !existing_resource.has_some_effect() {
						$new_resource.prefix_path_patterns = std::mem::take(&mut existing_resource.prefix_path_patterns);
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
	fn by_patterns_sub_resource_mut(
		&mut self,
		mut patterns: impl Iterator<Item = Pattern>,
	) -> &mut Resource {
		let mut leaf_resource_in_the_path = self.by_patterns_leaf_resource_mut(&mut patterns);
		leaf_resource_in_the_path.by_patterns_new_sub_resource_mut(patterns)
	}

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
				let new_sub_resource = Resource::with_pattern(pattern).unwrap();
				current_resource.add_sub_resource(new_sub_resource);
				current_resource =
					current_resource.by_patterns_leaf_resource_mut(&mut std::iter::once(pattern_clone));
			}
		}

		current_resource
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
								let dummy_resource = Resource::with_pattern_str("dummy").unwrap(); // TODO: Provide default constructor;
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

	#[inline]
	fn path_patterns(&self) -> Vec<Pattern> {
		let mut prefix_patterns = self.prefix_path_patterns.clone();
		prefix_patterns.push(self.pattern.clone());

		prefix_patterns
	}

	pub fn add_sub_resource_under(&mut self, route: &str, mut new_resource: Resource) {
		if !new_resource.prefix_path_patterns.is_empty() {
			let mut new_resource_prefix_path_patterns =
				std::mem::take(&mut new_resource.prefix_path_patterns).into_iter();

			self.check_path_segments_are_the_same(&mut new_resource_prefix_path_patterns);

			if route.is_empty() {
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

			let prefix_route_segments = RouteSegments::new(route);
			for (prefix_route_segment, _) in prefix_route_segments {
				let Some(prefix_path_segment_pattern) = new_resource_prefix_path_patterns.next() else {
					panic!("prefix path patterns must be the same with the path patterns of the parent")
				};

				let prefix_route_segment_pattern = Pattern::parse(prefix_route_segment).unwrap();
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

		if route.is_empty() {
			self.add_sub_resource(new_resource);
		} else {
			let sub_resource_to_be_parent = self.sub_resource_mut(route);
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

		leaf_resource_in_the_path.new_sub_resource_mut(segments)
	}

	fn leaf_resource(&self, patterns: &mut RouteSegments) -> &Resource {
		let mut existing_resource = self;

		for (segment, segment_index) in patterns.by_ref() {
			let pattern = Pattern::parse(segment).unwrap();

			match pattern {
				Pattern::Static(_) => {
					let some_position = existing_resource
						.static_resources
						.iter()
						.position(|resource| resource.pattern.compare(&pattern) == Similarity::Same);

					if let Some(position) = some_position {
						existing_resource = &existing_resource.static_resources[position];
					} else {
						patterns.revert_to_segment(segment_index);

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
						patterns.revert_to_segment(segment_index);

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
						patterns.revert_to_segment(segment_index);

						break;
					}
				}
			}
		}

		existing_resource
	}

	fn leaf_resource_mut(&mut self, patterns: &mut RouteSegments) -> &mut Resource {
		let mut existing_resource = self;

		for (segment, segment_index) in patterns.by_ref() {
			let pattern = Pattern::parse(segment).unwrap();

			match pattern {
				Pattern::Static(_) => {
					let some_position = existing_resource
						.static_resources
						.iter()
						.position(|resource| resource.pattern.compare(&pattern) == Similarity::Same);

					if let Some(position) = some_position {
						existing_resource = &mut existing_resource.static_resources[position];
					} else {
						patterns.revert_to_segment(segment_index);

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
						patterns.revert_to_segment(segment_index);

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
						patterns.revert_to_segment(segment_index);

						break;
					}
				}
			}
		}

		existing_resource
	}

	#[inline]
	fn new_sub_resource_mut(&mut self, segments: RouteSegments) -> &mut Resource {
		let mut current_resource = self;

		for (segment, _) in segments {
			let pattern = Pattern::parse(segment).unwrap();

			if let Some(name) = pattern.name() {
				if current_resource.path_has_the_same_name(name) {
					panic!("{} is not unique in the path", name)
				}

				let new_sub_resource = Resource::with_pattern(pattern).unwrap();
				current_resource.add_sub_resource(new_sub_resource);
				current_resource = current_resource.sub_resource_mut(segment);
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

	// TODO: Remove BoxedErrors.
	pub fn set_state<S: Clone + Send + Sync + 'static>(
		&mut self,
		state: S,
	) -> Result<(), BoxedError> {
		let state_type_id = state.type_id();

		if self
			.state
			.iter()
			.any(|existing_state| (*existing_state).type_id() == state_type_id)
		{
			return Err(BoxedError::from(format!(
				"resource already has a state of type '{:?}'",
				TypeId::of::<S>()
			)));
		}

		self.state.push(Box::new(state));

		Ok(())
	}

	pub fn state<S: Clone + Send + Sync + 'static>(&self) -> Result<&S, BoxedError> {
		self
			.state
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
	// pub fn config(&self) -> Config {
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

	pub fn set_sub_resource_state<S: Clone + Send + Sync + 'static>(
		&mut self,
		route: &str,
		state: S,
	) {
		let sub_resource = self.sub_resource_mut(route);
		sub_resource.set_state(state);
	}

	pub fn sub_resource_state<S: Clone + Send + Sync + 'static>(
		&self,
		route: &str,
	) -> Result<&S, BoxedError> {
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

	pub fn set_sub_resource_handler<H, M>(&mut self, route: &str, method: Method, handler: H)
	where
		H: IntoHandler<M, IncomingBody>,
		H::Handler: Handler + Send + Sync + 'static,
		<H::Handler as Handler>::Response: IntoResponse,
	{
		let sub_resource = self.sub_resource_mut(route);
		sub_resource.set_handler(method, handler);
	}

	pub fn wrap_sub_resource_request_receiver<L, LayeredB>(&mut self, route: &str, layer: L)
	where
		L: Layer<AdaptiveHandler<LayeredB>, LayeredB>,
		L::Handler: Handler + Send + Sync + 'static,
		<L::Handler as Handler>::Response: IntoResponse,
	{
		let sub_resource = self.sub_resource_mut(route);
		sub_resource.wrap_request_receiver(layer);
	}

	pub fn wrap_sub_resource_request_passer<L, LayeredB>(&mut self, route: &str, layer: L)
	where
		L: Layer<AdaptiveHandler<LayeredB>, LayeredB>,
		L::Handler: Handler + Send + Sync + 'static,
		<L::Handler as Handler>::Response: IntoResponse,
	{
		let sub_resource = self.sub_resource_mut(route);
		sub_resource.wrap_request_passer(layer);
	}

	pub fn wrap_sub_resource_request_handler<L, LayeredB>(&mut self, route: &str, layer: L)
	where
		L: Layer<AdaptiveHandler<LayeredB>, LayeredB>,
		L::Handler: Handler + Send + Sync + 'static,
		<L::Handler as Handler>::Response: IntoResponse,
	{
		let sub_resource = self.sub_resource_mut(route);
		sub_resource.wrap_request_handler(layer);
	}

	pub fn wrap_sub_resource_method_handler<L, LayeredB>(
		&mut self,
		route: &str,
		method: Method,
		layer: L,
	) where
		L: Layer<AdaptiveHandler<LayeredB>, LayeredB>,
		L::Handler: Handler + Send + Sync + 'static,
		<L::Handler as Handler>::Response: IntoResponse,
	{
		let sub_resource = self.sub_resource_mut(route);
		sub_resource.wrap_method_handler(method, layer);
	}

	// -------------------------

	pub fn set_sub_resources_state<S: Clone + Send + Sync + 'static>(&mut self, state: S) {
		self.call_for_each_sub_resource(|sub_resource| {
			sub_resource.set_state(state.clone());
		});
	}

	// pub fn set_sub_resources_config(&mut self, config: Config) {
	// 	todo!()
	// }

	pub fn wrap_sub_resources_request_receivers<L, LayeredB>(&mut self, layer: L)
	where
		L: Layer<AdaptiveHandler<LayeredB>, LayeredB> + Clone,
		L::Handler: Handler + Send + Sync + 'static,
		<L::Handler as Handler>::Response: IntoResponse,
	{
		self
			.call_for_each_sub_resource(|sub_resource| sub_resource.wrap_request_receiver(layer.clone()))
	}

	pub fn wrap_sub_resources_request_passers<L, LayeredB>(&mut self, layer: L)
	where
		L: Layer<AdaptiveHandler<LayeredB>, LayeredB> + Clone,
		L::Handler: Handler + Send + Sync + 'static,
		<L::Handler as Handler>::Response: IntoResponse,
	{
		self.call_for_each_sub_resource(|sub_resource| sub_resource.wrap_request_passer(layer.clone()))
	}

	pub fn wrap_sub_resources_request_handlers<L, LayeredB>(&mut self, layer: L)
	where
		L: Layer<AdaptiveHandler<LayeredB>, LayeredB> + Clone,
		L::Handler: Handler + Send + Sync + 'static,
		<L::Handler as Handler>::Response: IntoResponse,
	{
		self.call_for_each_sub_resource(|sub_resource| sub_resource.wrap_request_handler(layer.clone()))
	}

	pub fn wrap_sub_resources_method_handlers<L, LayeredB>(&mut self, method: Method, layer: L)
	where
		L: Layer<AdaptiveHandler<LayeredB>, LayeredB> + Clone,
		L::Handler: Handler + Send + Sync + 'static,
		<L::Handler as Handler>::Response: IntoResponse,
	{
		self.call_for_each_sub_resource(|sub_resource| {
			sub_resource.wrap_method_handler(method.clone(), layer.clone())
		})
	}

	fn call_for_each_sub_resource(&mut self, func: impl Fn(&mut Resource)) {
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

			func(sub_resource);

			sub_resources.extend(sub_resource.static_resources.iter_mut());
			sub_resources.extend(sub_resource.regex_resources.iter_mut());
			if let Some(resource) = sub_resource.wildcard_resource.as_deref_mut() {
				sub_resources.push(resource);
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
			state,
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
