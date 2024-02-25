use core::panic;
use std::{any, future::ready, num::NonZeroIsize, str::FromStr, sync::Arc};

use http::{Extensions, StatusCode, Uri};

use crate::{
	common::{BoxedError, BoxedFuture, IntoArray, SCOPE_VALIDITY},
	handler::{AdaptiveHandler, BoxedHandler, Handler},
	host::Host,
	middleware::{BoxedLayer, IntoLayer, Layer, RequestExtensionsModifierLayer},
	pattern::{Pattern, Similarity},
	resource::{Iteration, Resource},
	response::{BoxedErrorResponse, Response},
};

// --------------------------------------------------

mod service;

pub use service::RouterService;

use self::service::RequestPasser;

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

pub struct Router {
	static_hosts: Vec<Host>,
	regex_hosts: Vec<Host>,
	some_root_resource: Option<Box<Resource>>,

	extensions: Extensions,
	middleware: Vec<RouterLayerTarget>,
}

impl Router {
	pub fn new() -> Router {
		Self {
			static_hosts: Vec::new(),
			regex_hosts: Vec::new(),
			some_root_resource: None,

			extensions: Extensions::new(),
			middleware: Vec::new(),
		}
	}

	pub fn add_resource<R, const N: usize>(&mut self, new_resources: R)
	where
		R: IntoArray<Resource, N>,
	{
		let new_resources = new_resources.into_array();

		for new_resource in new_resources {
			self.add_single_resource(new_resource)
		}
	}

	fn add_single_resource(&mut self, mut new_resource: Resource) {
		if let Some(host) = new_resource
			.host_pattern_ref()
			.and_then(|host_pattern| self.existing_host_mut(host_pattern))
		{
			if new_resource.pattern_string() == "/" {
				host.merge_or_replace_root(new_resource);
			} else {
				host.root_mut().add_subresource(new_resource);
			}

			return;
		}

		if let Some(host_pattern) = new_resource.host_pattern_ref().map(Clone::clone) {
			let root = if new_resource.pattern_string() == "/" {
				new_resource
			} else {
				let mut root = Resource::with_uri_patterns(
					Some(host_pattern.clone()),
					Vec::new(),
					Pattern::parse("/"),
					false,
				);

				root.add_subresource(new_resource);

				root
			};

			self.add_new_host(host_pattern, root);

			return;
		}

		if new_resource.pattern_string() == "/" {
			self.merge_or_replace_root(new_resource);
		} else {
			if let Some(boxed_root) = self.some_root_resource.as_mut() {
				boxed_root.add_subresource(new_resource);
			} else {
				let mut root = Resource::with_pattern(Pattern::parse("/"));
				root.add_subresource(new_resource);

				self.some_root_resource = Some(Box::new(root));
			}
		}
	}

	fn existing_host_mut(&mut self, host_pattern: &Pattern) -> Option<&mut Host> {
		match host_pattern {
			Pattern::Static(_) => self
				.static_hosts
				.iter_mut()
				.find(|static_host| static_host.compare_pattern(host_pattern) == Similarity::Same),
			Pattern::Regex(_, _) => self
				.regex_hosts
				.iter_mut()
				.find(|regex_host| regex_host.compare_pattern(host_pattern) == Similarity::Same),
			Pattern::Wildcard(_) => unreachable!(),
		}
	}

	fn add_new_host(&mut self, host_pattern: Pattern, root: Resource) {
		let mut host = match host_pattern {
			Pattern::Static(_) => &mut self.static_hosts,
			Pattern::Regex(_, _) => &mut self.regex_hosts,
			Pattern::Wildcard(_) => unreachable!(),
		};

		host.push(Host::with_pattern(host_pattern, root));
	}

	fn merge_or_replace_root(&mut self, mut new_root: Resource) {
		if let Some(mut boxed_root) = self.some_root_resource.take() {
			if !new_root.has_some_effect() {
				boxed_root.keep_subresources(new_root);
			} else if !boxed_root.has_some_effect() {
				new_root.keep_subresources(*boxed_root);
				*boxed_root = new_root;
			} else {
				panic!("conflicting root resources")
			}

			self.some_root_resource = Some(boxed_root);
		} else {
			self.some_root_resource = Some(Box::new(new_root));
		}
	}

	pub fn add_resource_under<U, R, const N: usize>(&mut self, uri_pattern: U, new_resources: R)
	where
		U: AsRef<str>,
		R: IntoArray<Resource, N>,
	{
		let uri_pattern = Uri::from_str(uri_pattern.as_ref()).expect("invalid URI pattern");
		let new_resources = new_resources.into_array();

		for new_resource in new_resources {
			self.add_single_resource_under(&uri_pattern, new_resource)
		}
	}

	fn add_single_resource_under(&mut self, uri_pattern: &Uri, new_resource: Resource) {
		let some_host_pattern = uri_pattern
			.host()
			.map(|host_pattern| Pattern::parse(host_pattern));
		let path_patterns = uri_pattern.path();

		let new_resource_is_root = new_resource.pattern_string() == "/";

		if new_resource_is_root {
			if !path_patterns.is_empty() {
				panic!("a root resource cannot be a subresource");
			}
		}

		if let Some(host_pattern) = some_host_pattern {
			if let Some(host) = self.existing_host_mut(&host_pattern) {
				if new_resource_is_root {
					host.merge_or_replace_root(new_resource);
				} else {
					host
						.root_mut()
						.add_subresource_under(path_patterns, new_resource);
				}

				return;
			}

			let root = if new_resource_is_root {
				new_resource
			} else {
				let mut root = Resource::with_uri_patterns(
					Some(host_pattern.clone()),
					Vec::new(),
					Pattern::parse("/"),
					false,
				);

				root.add_subresource_under(path_patterns, new_resource);

				root
			};

			self.add_new_host(host_pattern, root);

			return;
		}

		if new_resource_is_root {
			self.merge_or_replace_root(new_resource);
		} else {
			if let Some(boxed_root) = self.some_root_resource.as_mut() {
				boxed_root.add_subresource_under(path_patterns, new_resource);
			} else {
				let mut root = Resource::with_pattern(Pattern::parse("/"));
				root.add_subresource_under(path_patterns, new_resource);

				self.some_root_resource = Some(Box::new(root));
			}
		}
	}

	pub fn resource_mut<U>(&mut self, uri_pattern: U) -> &mut Resource
	where
		U: AsRef<str>,
	{
		let uri_pattern = Uri::from_str(uri_pattern.as_ref()).expect("invalid URI pattern");
		let resource_is_root = uri_pattern.path() == "/";

		if let Some(host_pattern) = uri_pattern.host().map(Pattern::parse) {
			let new_host =
				match &host_pattern {
					Pattern::Static(_) => {
						if let Some(position) = self.static_hosts.iter().position(|static_host| {
							static_host.compare_pattern(&host_pattern) == Similarity::Same
						}) {
							return if resource_is_root || uri_pattern.path().is_empty() {
								self.static_hosts[position].root_mut()
							} else {
								self.static_hosts[position]
									.root_mut()
									.subresource_mut(uri_pattern.path())
							};
						}

						self.static_hosts.push(Host::with_pattern(
							host_pattern,
							Resource::with_pattern(Pattern::parse("/")),
						));

						self.static_hosts.last_mut().expect(SCOPE_VALIDITY)
					}
					Pattern::Regex(_, _) => {
						if let Some(position) = self
							.regex_hosts
							.iter()
							.position(|regex_host| regex_host.compare_pattern(&host_pattern) == Similarity::Same)
						{
							return if resource_is_root || uri_pattern.path().is_empty() {
								self.regex_hosts[position].root_mut()
							} else {
								self.regex_hosts[position]
									.root_mut()
									.subresource_mut(uri_pattern.path())
							};
						}

						self.regex_hosts.push(Host::with_pattern(
							host_pattern,
							Resource::with_pattern(Pattern::parse("/")),
						));

						self.regex_hosts.last_mut().expect(SCOPE_VALIDITY)
					}
					Pattern::Wildcard(_) => unreachable!(),
				};

			if resource_is_root || uri_pattern.path().is_empty() {
				return new_host.root_mut();
			}

			return new_host.root_mut().subresource_mut(uri_pattern.path());
		}

		if uri_pattern.path().is_empty() {
			panic!("invalid URI pattern");
		}

		if self.some_root_resource.is_none() {
			self.some_root_resource = Some(Box::new(Resource::with_pattern(Pattern::parse("/"))));
		}

		let root = self
			.some_root_resource
			.as_deref_mut()
			.expect(SCOPE_VALIDITY);

		if resource_is_root {
			root
		} else {
			root.subresource_mut(uri_pattern.path())
		}
	}

	pub fn add_extension<E: Clone + Send + Sync + 'static>(&mut self, extension: E) {
		if self.extensions.insert(extension).is_some() {
			panic!(
				"router already has an extension of type '{}'",
				any::type_name::<E>()
			);
		}
	}

	pub fn add_layer<L, const N: usize>(&mut self, layer_targets: L)
	where
		L: IntoArray<RouterLayerTarget, N>,
	{
		self.middleware.extend(layer_targets.into_array());
	}

	pub fn set_config<C, const N: usize>(&mut self, config_options: C)
	where
		C: IntoArray<RouterConfigOption, N>,
	{
		let config_options = config_options.into_array();

		for config_option in config_options {
			let RouterConfigOptionValue(request_extensions_modifier_layer) = config_option.0;
			let request_passer_layer_target = request_passer(request_extensions_modifier_layer);

			self.middleware.insert(0, request_passer_layer_target);
		}
	}

	pub fn for_each_root<T, F>(&mut self, mut param: T, mut func: F) -> T
	where
		F: FnMut(&mut T, Option<&str>, &mut Resource) -> Iteration,
	{
		let mut root_resources = Vec::new();
		root_resources.extend(
			self
				.static_hosts
				.iter_mut()
				.map(|static_host| (Some(static_host.pattern_string()), static_host.root_mut())),
		);

		root_resources.extend(
			self
				.regex_hosts
				.iter_mut()
				.map(|regex_host| (Some(regex_host.pattern_string()), regex_host.root_mut())),
		);

		if let Some(root) = self.some_root_resource.as_deref_mut() {
			root_resources.push((None, root));
		}

		loop {
			let Some((some_host_pattern, root)) = root_resources.pop() else {
				break param;
			};

			match func(
				&mut param,
				some_host_pattern
					.as_ref()
					.map(|host_pattern| host_pattern.as_str()),
				root,
			) {
				Iteration::Stop => break param,
				_ => {}
			}
		}
	}

	pub fn into_service(self) -> RouterService {
		let Router {
			static_hosts,
			regex_hosts,
			some_root_resource,
			extensions,
			middleware,
		} = self;

		let some_static_hosts = if static_hosts.is_empty() {
			None
		} else {
			Some(
				static_hosts
					.into_iter()
					.map(|static_host| static_host.into_service())
					.collect(),
			)
		};

		let some_regex_hosts = if regex_hosts.is_empty() {
			None
		} else {
			Some(
				regex_hosts
					.into_iter()
					.map(|regex_host| regex_host.into_service())
					.collect(),
			)
		};

		let some_root_resource =
			some_root_resource.map(|root_resource| Arc::new(root_resource.into_service()));

		let request_passer = RequestPasser::new(
			some_static_hosts,
			some_regex_hosts,
			some_root_resource,
			middleware,
		);

		RouterService::new(extensions, request_passer)
	}
}

// --------------------------------------------------
// RouterLayerTarget

pub struct RouterLayerTarget(RouterLayerTargetValue);

#[derive(Default)]
enum RouterLayerTargetValue {
	#[default]
	None,
	RequestPasser(BoxedLayer),
}

impl RouterLayerTargetValue {
	#[inline(always)]
	fn take(&mut self) -> Self {
		std::mem::take(self)
	}
}

// ----------

pub fn request_passer<L, M>(layer: L) -> RouterLayerTarget
where
	L: IntoLayer<M, AdaptiveHandler>,
	L::Layer: Layer<AdaptiveHandler> + Clone + 'static,
	<L::Layer as Layer<AdaptiveHandler>>::Handler: Handler<
			Response = Response,
			Error = BoxedErrorResponse,
			Future = BoxedFuture<Result<Response, BoxedErrorResponse>>,
		> + Clone
		+ Send
		+ Sync
		+ 'static,
{
	RouterLayerTarget(RouterLayerTargetValue::RequestPasser(BoxedLayer::new(
		layer.into_layer(),
	)))
}

// --------------------------------------------------
// RouterConfigOption

pub struct RouterConfigOption(RouterConfigOptionValue);

struct RouterConfigOptionValue(RequestExtensionsModifierLayer);

impl IntoArray<RouterConfigOption, 1> for RouterConfigOption {
	fn into_array(self) -> [RouterConfigOption; 1] {
		[self]
	}
}

// ----------

pub fn modify_request_extensions<Func>(modifier: Func) -> RouterConfigOption
where
	Func: Fn(&mut Extensions) + Clone + Send + Sync + 'static,
{
	let request_extensions_modifier_layer = RequestExtensionsModifierLayer::new(modifier);

	RouterConfigOption(RouterConfigOptionValue(request_extensions_modifier_layer))
}

// --------------------------------------------------------------------------------
