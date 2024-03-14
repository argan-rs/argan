use core::panic;
use std::{any, future::ready, num::NonZeroIsize, str::FromStr, sync::Arc};

use http::{Extensions, StatusCode, Uri};

use crate::{
	common::{BoxedError, BoxedFuture, IntoArray, SCOPE_VALIDITY},
	handler::{AdaptiveHandler, BoxedHandler, Handler},
	host::Host,
	middleware::{BoxedLayer, IntoLayer, Layer, RequestExtensionsModifierLayer},
	pattern::{split_uri_host_and_path, Pattern, Similarity},
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
			if new_resource.is("/") {
				host.merge_or_replace_root(new_resource);
			} else {
				host.root_mut().add_subresource(new_resource);
			}

			return;
		}

		if let Some(host_pattern) = new_resource.host_pattern_ref().map(Clone::clone) {
			let root = if new_resource.is("/") {
				new_resource
			} else {
				let mut root = Resource::with_uri_pattern(
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

		if new_resource.is("/") {
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
		let (some_host_pattern_str, some_path_pattern_str) =
			split_uri_host_and_path(uri_pattern.as_ref());

		let new_resources = new_resources.into_array();

		for new_resource in new_resources {
			self.add_single_resource_under(some_host_pattern_str, some_path_pattern_str, new_resource)
		}
	}

	fn add_single_resource_under(
		&mut self,
		some_host_pattern_str: Option<&str>,
		some_path_pattern_str: Option<&str>,
		new_resource: Resource,
	) {
		let some_host_pattern = some_host_pattern_str.map(|host_pattern| Pattern::parse(host_pattern));

		let new_resource_is_root = new_resource.is("/");

		if new_resource_is_root && some_path_pattern_str.is_some() {
			panic!("a root resource cannot be a subresource of another resource");
		}

		let relative_path_pattern_str = some_path_pattern_str.unwrap_or("");

		if let Some(host_pattern) = some_host_pattern {
			if let Some(host) = self.existing_host_mut(&host_pattern) {
				if new_resource_is_root {
					host.merge_or_replace_root(new_resource);
				} else {
					let root = host.root_mut();
					if relative_path_pattern_str == "/" {
						root.add_subresource(new_resource);
					} else {
						root.add_subresource_under(relative_path_pattern_str, new_resource);
					}
				}

				return;
			}

			let root = if new_resource_is_root {
				new_resource
			} else {
				let mut root = Resource::with_uri_pattern(
					Some(host_pattern.clone()),
					Vec::new(),
					Pattern::parse("/"),
					false,
				);

				if relative_path_pattern_str == "/" {
					root.add_subresource(new_resource);
				} else {
					root.add_subresource_under(relative_path_pattern_str, new_resource);
				}

				root
			};

			self.add_new_host(host_pattern, root);

			return;
		}

		if new_resource_is_root {
			self.merge_or_replace_root(new_resource);
		} else {
			let boxed_root = if let Some(boxed_root) = self.some_root_resource.as_mut() {
				boxed_root
			} else {
				let mut root = Resource::with_pattern(Pattern::parse("/"));
				self.some_root_resource = Some(Box::new(root));

				self.some_root_resource.as_mut().expect(SCOPE_VALIDITY)
			};

			if relative_path_pattern_str == "/" {
				boxed_root.add_subresource(new_resource);
			} else {
				boxed_root.add_subresource_under(relative_path_pattern_str, new_resource);
			}
		}
	}

	pub fn resource_mut<U>(&mut self, uri_pattern: U) -> &mut Resource
	where
		U: AsRef<str>,
	{
		let (some_host_pattern_str, Some(path_pattern_str)) =
			split_uri_host_and_path(uri_pattern.as_ref())
		else {
			panic!("empty path pattern");
		};

		let resource_is_root = path_pattern_str == "/";

		if let Some(host_pattern) = some_host_pattern_str.map(Pattern::parse) {
			let new_host =
				match &host_pattern {
					Pattern::Static(_) => {
						if let Some(position) = self.static_hosts.iter().position(|static_host| {
							static_host.compare_pattern(&host_pattern) == Similarity::Same
						}) {
							return if resource_is_root {
								self.static_hosts[position].root_mut()
							} else {
								self.static_hosts[position]
									.root_mut()
									.subresource_mut(path_pattern_str)
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
							return if resource_is_root {
								self.regex_hosts[position].root_mut()
							} else {
								self.regex_hosts[position]
									.root_mut()
									.subresource_mut(path_pattern_str)
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

			if resource_is_root {
				return new_host.root_mut();
			}

			return new_host.root_mut().subresource_mut(path_pattern_str);
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
			root.subresource_mut(path_pattern_str)
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

	pub fn add_layer_to<L, const N: usize>(&mut self, layer_targets: L)
	where
		L: IntoArray<RouterLayerTarget, N>,
	{
		self.middleware.extend(layer_targets.into_array());
	}

	pub fn configure<C, const N: usize>(&mut self, config_options: C)
	where
		C: IntoArray<RouterConfigOption, N>,
	{
		let config_options = config_options.into_array();

		for config_option in config_options {
			let RouterConfigOption(request_extensions_modifier_layer) = config_option;
			let request_passer_layer_target = _request_passer(request_extensions_modifier_layer);

			self.middleware.insert(0, request_passer_layer_target);
		}
	}

	pub fn for_each_root<T, F>(&mut self, mut param: T, mut func: F) -> T
	where
		F: FnMut(&mut T, &mut Resource) -> Iteration,
	{
		let mut root_resources = Vec::new();
		root_resources.extend(
			self
				.static_hosts
				.iter_mut()
				.map(|static_host| static_host.root_mut()),
		);

		root_resources.extend(
			self
				.regex_hosts
				.iter_mut()
				.map(|regex_host| regex_host.root_mut()),
		);

		if let Some(root) = self.some_root_resource.as_deref_mut() {
			root_resources.push(root);
		}

		loop {
			let Some(root) = root_resources.pop() else {
				break param;
			};

			match func(&mut param, root) {
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
// RouterConfigOption

pub mod config {
	use super::*;

	mod private {
		use super::*;

		pub struct RouterConfigOption(pub(crate) RequestExtensionsModifierLayer);

		impl IntoArray<RouterConfigOption, 1> for RouterConfigOption {
			fn into_array(self) -> [RouterConfigOption; 1] {
				[self]
			}
		}
	}

	pub(super) use private::RouterConfigOption;

	pub fn _to_modify_request_extensions<Func>(modifier: Func) -> RouterConfigOption
	where
		Func: Fn(&mut Extensions) + Clone + Send + Sync + 'static,
	{
		let request_extensions_modifier_layer = RequestExtensionsModifierLayer::new(modifier);

		RouterConfigOption(request_extensions_modifier_layer)
	}
}

use config::RouterConfigOption;
pub use config::_to_modify_request_extensions;

// --------------------------------------------------
// RouterLayerTarget

pub mod layer_targets {
	use super::*;

	mod private {
		use super::*;

		#[allow(private_interfaces)]
		#[derive(Default)]
		pub enum RouterLayerTarget {
			#[default]
			None,
			RequestPasser(BoxedLayer),
		}

		impl RouterLayerTarget {
			#[inline(always)]
			pub(crate) fn take(&mut self) -> Self {
				std::mem::take(self)
			}
		}
	}

	pub(super) use private::RouterLayerTarget;

	// ----------

	pub fn _request_passer<L, M>(layer: L) -> RouterLayerTarget
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
		RouterLayerTarget::RequestPasser(BoxedLayer::new(layer.into_layer()))
	}
}

use layer_targets::RouterLayerTarget;
pub use layer_targets::_request_passer;

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

#[cfg(test)]
mod test {
	use crate::{
		handler::_get,
		resource::config::{_as_subtree_handler, _to_drop_on_unmatching_slash},
	};

	use super::*;

	#[test]
	fn router_add_resource() {
		//	http://example_0.com	->	/st_0_0	->	/{wl_1_0}			->	/{rx_2_0:p0}	->	/st_3_0
		//											|						|
		//											|						|	->	/{rx_1_1:p0}	->	/{wl_2_0}
		//											|															|	->	/st_2_1
		//											|															|	->	/st_2_2
		//											|
		//											|	->	/st_0_1	->	/{wl_1_0}	->	/{rx_2_0:p0}
		//																								|	->	/{rx_2_1:p1}

		//	http://{sub}.example_1.com	->	/st_0_0	->	/{wl_1_0}			->	/{rx_2_0:p0}	->	/st_3_0
		//														|						|
		//														|						|	->	/{rx_1_1:p0}	->	/{wl_2_0}
		//														|															|	->	/st_2_1
		//														|															|	->	/st_2_2
		//														|
		//														|	->	/st_0_1	->	/{wl_1_0}	->	/{rx_2_0:p0}
		//																											|	->	/{rx_2_1:p1}

		//	/	->	/st_0_0	->	/{wl_1_0}			->	/{rx_2_0:p0}	->	/st_3_0
		//	|						|
		//	|						|	->	/{rx_1_1:p0}	->	/{wl_2_0}
		//	|															|	->	/st_2_1
		//	|															|	->	/st_2_2
		//	|
		//	|	->	/st_0_1	->	/{wl_1_0}	->	/{rx_2_0:p0}
		//														|	->	/{rx_2_1:p1}

		let mut router = Router::new();

		let cases = [
			"http://example_0.com/st_0_0/{wl_1_0}/{rx_2_0:p0}/st_3_0",
			"http://example_0.com/st_0_0/{rx_1_1:p0}/{wl_2_0}",
			"http://example_0.com/st_0_0/{rx_1_1:p0}/st_2_1",
			"http://example_0.com/st_0_0/{rx_1_1:p0}/st_2_1",
			"http://example_0.com/st_0_1/{wl_1_0}/{rx_2_0:p0}",
			"http://example_0.com/st_0_1/{wl_1_0}/{rx_2_1:p1}",
			// -----
			"http://{sub}.example_1.com/st_0_0/{wl_1_0}/{rx_2_0:p0}/st_3_0",
			"http://{sub}.example_1.com/st_0_0/{rx_1_1:p0}/{wl_2_0}",
			"http://{sub}.example_1.com/st_0_0/{rx_1_1:p0}/st_2_1",
			"http://{sub}.example_1.com/st_0_0/{rx_1_1:p0}/st_2_1",
			"http://{sub}.example_1.com/st_0_1/{wl_1_0}/{rx_2_0:p0}",
			"http://{sub}.example_1.com/st_0_1/{wl_1_0}/{rx_2_1:p1}",
			// -----
			"/st_0_0/{wl_1_0}/{rx_2_0:p0}/st_3_0",
			"/st_0_0/{rx_1_1:p0}/{wl_2_0}",
			"/st_0_0/{rx_1_1:p0}/st_2_1",
			"/st_0_0/{rx_1_1:p0}/st_2_1",
			"/st_0_1/{wl_1_0}/{rx_2_0:p0}",
			"/st_0_1/{wl_1_0}/{rx_2_1:p1}",
		];

		for case in cases {
			dbg!(case);

			let resource = Resource::new(case);
			router.add_resource(resource);
		}

		// ----------

		dbg!();

		{
			assert_eq!(router.static_hosts.len(), 1);
			let example_com = router
				.existing_host_mut(&Pattern::parse("example_0.com"))
				.unwrap();

			assert_eq!(example_com.root_mut().static_resources().len(), 2);
		}

		{
			assert_eq!(router.regex_hosts.len(), 1);
			let sub_example_com = router
				.existing_host_mut(&Pattern::parse("{sub}.example_1.com"))
				.unwrap();

			assert_eq!(sub_example_com.root_mut().static_resources().len(), 2);
		}

		{
			let root = router.some_root_resource.as_ref().unwrap();
			assert_eq!(root.static_resources().len(), 2);
		}

		// ----------

		dbg!();

		router.add_resource(Resource::new("http://example_0.com/{wl_0_2}"));
		router.add_resource(Resource::new("http://{sub}.example_1.com/{wl_0_2}"));
		router.add_resource(Resource::new("http://{sub}.example_2.com/{rx_0_0:p0}"));
		router.add_resource(Resource::new("/{wl_0_2}"));

		{
			assert_eq!(router.static_hosts.len(), 1);
			let example_0_com = router
				.existing_host_mut(&Pattern::parse("example_0.com"))
				.unwrap();

			let root = example_0_com.root_mut();
			assert_eq!(root.static_resources().len(), 2);
			assert_eq!(root.regex_resources().len(), 0);
			assert!(root.wildcard_resources().is_some());
		}

		{
			assert_eq!(router.regex_hosts.len(), 2);
			let sub_example_1_com = router
				.existing_host_mut(&Pattern::parse("{sub}.example_1.com"))
				.unwrap();

			let root = sub_example_1_com.root_mut();
			assert_eq!(root.static_resources().len(), 2);
			assert_eq!(root.regex_resources().len(), 0);
			assert!(root.wildcard_resources().is_some());
		}

		{
			let sub_example_2_com = router
				.existing_host_mut(&Pattern::parse("{sub}.example_2.com"))
				.unwrap();

			let root = sub_example_2_com.root_mut();
			assert_eq!(root.static_resources().len(), 0);
			assert_eq!(root.regex_resources().len(), 1);
			assert!(root.wildcard_resources().is_none());
		}

		{
			let root = router.some_root_resource.as_ref().unwrap();
			assert_eq!(root.static_resources().len(), 2);
			assert_eq!(root.regex_resources().len(), 0);
			assert!(root.wildcard_resources().is_some());
		}

		// ----------

		dbg!();

		{
			let mut new_root = Resource::new("http://example_0.com/");
			new_root.set_handler_for(_get(|| async {}));
			router.add_resource(new_root);

			let example_0_com = router
				.existing_host_mut(&Pattern::parse("example_0.com"))
				.unwrap();

			let root = example_0_com.root_mut();
			assert!(root.can_handle_request());
			assert_eq!(root.static_resources().len(), 2);
			assert_eq!(root.regex_resources().len(), 0);
			assert!(root.wildcard_resources().is_some());
		}

		{
			let mut new_root = Resource::new("/");
			new_root.set_handler_for(_get(|| async {}));
			router.add_resource(new_root);

			let root = router.some_root_resource.as_ref().unwrap();
			assert!(root.can_handle_request());
			assert_eq!(root.static_resources().len(), 2);
			assert_eq!(root.regex_resources().len(), 0);
			assert!(root.wildcard_resources().is_some());
		}
	}

	#[test]
	fn router_add_resource_under() {
		//	http://example_0.com	->	/st_0_0	->	/{wl_1_0}			->	/{rx_2_0:p0}	->	/st_3_0
		//											|						|
		//											|						|	->	/{rx_1_1:p0}	->	/{wl_2_0}
		//											|															|	->	/st_2_1
		//											|															|	->	/st_2_2
		//											|
		//											|	->	/st_0_1	->	/{wl_1_0}	->	/{rx_2_0:p0}
		//																								|	->	/{rx_2_1:p1}

		//	http://{sub}.example_1.com	->	/st_0_0	->	/{wl_1_0}			->	/{rx_2_0:p0}	->	/st_3_0
		//														|						|
		//														|						|	->	/{rx_1_1:p0}	->	/{wl_2_0}
		//														|															|	->	/st_2_1
		//														|															|	->	/st_2_2
		//														|
		//														|	->	/st_0_1	->	/{wl_1_0}	->	/{rx_2_0:p0}
		//																											|	->	/{rx_2_1:p1}

		//	/	->	/st_0_0	->	/{wl_1_0}			->	/{rx_2_0:p0}	->	/st_3_0
		//	|						|
		//	|						|	->	/{rx_1_1:p0}	->	/{wl_2_0}
		//	|															|	->	/st_2_1
		//	|															|	->	/st_2_2
		//	|
		//	|	->	/st_0_1	->	/{wl_1_0}	->	/{rx_2_0:p0}
		//														|	->	/{rx_2_1:p1}

		let mut router = Router::new();

		#[derive(Debug)]
		struct Case {
			prefix_uri: &'static str,
			resource_uri: &'static str,
		}

		let cases = [
			Case {
				prefix_uri: "http://example_0.com/st_0_0/{wl_1_0}/{rx_2_0:p0}",
				resource_uri: "/st_3_0",
			},
			Case {
				prefix_uri: "http://example_0.com/st_0_0/",
				resource_uri: "/st_0_0/{rx_1_1:p0}/{wl_2_0}",
			},
			Case {
				prefix_uri: "http://example_0.com/st_0_0/{rx_1_1:p0}",
				resource_uri: "/st_2_1",
			},
			Case {
				prefix_uri: "http://example_0.com/st_0_0/{rx_1_1:p0}/",
				resource_uri: "http://example_0.com/st_0_0/{rx_1_1:p0}/st_2_2",
			},
			Case {
				prefix_uri: "https://example_0.com/",
				resource_uri: "http://example_0.com/st_0_1/{wl_1_0}/{rx_2_0:p0}/",
			},
			Case {
				prefix_uri: "https://example_0.com/st_0_1/",
				resource_uri: "http://example_0.com/st_0_1/{wl_1_0}/{rx_2_1:p1}/",
			},
			// -----
			Case {
				prefix_uri: "http://{sub}.example_1.com/st_0_0/{wl_1_0}/{rx_2_0:p0}",
				resource_uri: "/st_3_0",
			},
			Case {
				prefix_uri: "http://{sub}.example_1.com/st_0_0/",
				resource_uri: "/st_0_0/{rx_1_1:p0}/{wl_2_0}",
			},
			Case {
				prefix_uri: "http://{sub}.example_1.com/st_0_0/{rx_1_1:p0}",
				resource_uri: "/st_2_1",
			},
			Case {
				prefix_uri: "http://{sub}.example_1.com/st_0_0/{rx_1_1:p0}",
				resource_uri: "http://{sub}.example_1.com/st_0_0/{rx_1_1:p0}/st_2_2",
			},
			Case {
				prefix_uri: "https://{sub}.example_1.com/",
				resource_uri: "http://{sub}.example_1.com/st_0_1/{wl_1_0}/{rx_2_0:p0}/",
			},
			Case {
				prefix_uri: "https://{sub}.example_1.com/st_0_1/",
				resource_uri: "http://{sub}.example_1.com/st_0_1/{wl_1_0}/{rx_2_1:p1}/",
			},
			// -----
			Case {
				prefix_uri: "/st_0_0/{wl_1_0}/{rx_2_0:p0}",
				resource_uri: "/st_3_0",
			},
			Case {
				prefix_uri: "/st_0_0/",
				resource_uri: "/st_0_0/{rx_1_1:p0}/{wl_2_0}",
			},
			Case {
				prefix_uri: "/st_0_0/{rx_1_1:p0}",
				resource_uri: "/st_2_1",
			},
			Case {
				prefix_uri: "/st_0_0/{rx_1_1:p0}",
				resource_uri: "/st_0_0/{rx_1_1:p0}/st_2_2",
			},
			Case {
				prefix_uri: "/",
				resource_uri: "/st_0_1/{wl_1_0}/{rx_2_0:p0}/",
			},
			Case {
				prefix_uri: "/st_0_1/",
				resource_uri: "/st_0_1/{wl_1_0}/{rx_2_1:p1}/",
			},
		];

		for case in &cases {
			dbg!(case);

			let resource = Resource::new(case.resource_uri);
			router.add_resource_under(case.prefix_uri, resource);
		}

		// ----------

		dbg!();

		{
			assert_eq!(router.static_hosts.len(), 1);
			let example_com = router
				.existing_host_mut(&Pattern::parse("example_0.com"))
				.unwrap();

			assert_eq!(example_com.root_mut().static_resources().len(), 2);
		}

		{
			assert_eq!(router.regex_hosts.len(), 1);
			let sub_example_com = router
				.existing_host_mut(&Pattern::parse("{sub}.example_1.com"))
				.unwrap();

			assert_eq!(sub_example_com.root_mut().static_resources().len(), 2);
		}

		{
			let root = router.some_root_resource.as_ref().unwrap();
			assert_eq!(root.static_resources().len(), 2);
		}

		// ----------

		dbg!();

		router.add_resource_under("http://example_0.com/", Resource::new("/{wl_0_2}"));
		router.add_resource_under(
			"http://{sub}.example_1.com/",
			Resource::new("http://{sub}.example_1.com/{wl_0_2}"),
		);

		router.add_resource_under(
			"http://{sub}.example_2.com/",
			Resource::new("http://{sub}.example_2.com/{rx_0_0:p0}"),
		);

		router.add_resource_under("/", Resource::new("/{wl_0_2}"));

		{
			assert_eq!(router.static_hosts.len(), 1);
			let example_0_com = router
				.existing_host_mut(&Pattern::parse("example_0.com"))
				.unwrap();

			let root = example_0_com.root_mut();
			assert_eq!(root.static_resources().len(), 2);
			assert_eq!(root.regex_resources().len(), 0);
			assert!(root.wildcard_resources().is_some());
		}

		{
			assert_eq!(router.regex_hosts.len(), 2);
			let sub_example_1_com = router
				.existing_host_mut(&Pattern::parse("{sub}.example_1.com"))
				.unwrap();

			let root = sub_example_1_com.root_mut();
			assert_eq!(root.static_resources().len(), 2);
			assert_eq!(root.regex_resources().len(), 0);
			assert!(root.wildcard_resources().is_some());
		}

		{
			let sub_example_2_com = router
				.existing_host_mut(&Pattern::parse("{sub}.example_2.com"))
				.unwrap();

			let root = sub_example_2_com.root_mut();
			assert_eq!(root.static_resources().len(), 0);
			assert_eq!(root.regex_resources().len(), 1);
			assert!(root.wildcard_resources().is_none());
		}

		{
			let root = router.some_root_resource.as_ref().unwrap();
			assert_eq!(root.static_resources().len(), 2);
			assert_eq!(root.regex_resources().len(), 0);
			assert!(root.wildcard_resources().is_some());
		}

		// ----------

		dbg!();

		{
			let mut new_root = Resource::new("http://example_0.com/");
			new_root.set_handler_for(_get(|| async {}));
			router.add_resource(new_root);

			let example_0_com = router
				.existing_host_mut(&Pattern::parse("example_0.com"))
				.unwrap();

			let root = example_0_com.root_mut();
			assert!(root.can_handle_request());
			assert_eq!(root.static_resources().len(), 2);
			assert_eq!(root.regex_resources().len(), 0);
			assert!(root.wildcard_resources().is_some());
		}

		{
			let mut new_root = Resource::new("/");
			new_root.set_handler_for(_get(|| async {}));
			router.add_resource(new_root);

			let root = router.some_root_resource.as_ref().unwrap();
			assert!(root.can_handle_request());
			assert_eq!(root.static_resources().len(), 2);
			assert_eq!(root.regex_resources().len(), 0);
			assert!(root.wildcard_resources().is_some());
		}
	}

	#[test]
	fn router_resource_mut() {
		//	http://example_0.com	->	/st_0_0	->	/{wl_1_0}	->	/{rx_2_0:p0}	->	/st_3_0
		//																								|
		//																								|	->	/{rx_2_1:p1}	->	/{wl_3_0}
		//																																	|	->	/st_3_1

		//	http://{sub}.example_1.com	/	->	/st_0_0	->	/{wl_1_0}	->	/{rx_2_0:p0}	->	/st_3_0
		//																												|
		//																												|	->	/{rx_2_1:p1}	->	/{wl_3_0}
		//																																					|	->	/st_3_1

		//	http://{sub}.example_2.com	/	->	/st_0_0	->	/{wl_1_0}	->	/{rx_2_0:p0}	->	/st_3_0
		//																												|
		//																												|	->	/{rx_2_1:p1}	->	/{wl_3_0}
		//																																					|	->	/st_3_1

		//	/	->	/st_0_0	->	/{wl_1_0}	->	/{rx_2_0:p0}	->	/st_3_0
		//														|
		//														|	->	/{rx_2_1:p1}	->	/{wl_3_0}
		//																							|	->	/st_3_1

		let handler = || async {};
		let mut router = Router::new();

		let cases = [
			"https://example_0.com/",
			"https://example_0.com/st_0_0",
			"http://example_0.com/st_0_0/{wl_1_0}/{rx_2_0:p0}/st_3_0",
			"http://example_0.com/st_0_0/{wl_1_0}/{rx_2_1:p1}/{wl_3_0}/",
			// -----
			"https://{sub}.example_1.com/",
			"https://{sub}.example_1.com/st_0_0",
			"http://{sub}.example_1.com/st_0_0/{wl_1_0}/{rx_2_0:p0}/st_3_0",
			"http://{sub}.example_1.com/st_0_0/{wl_1_0}/{rx_2_1:p1}/{wl_3_0}/",
			// -----
			"https://{sub}.example_2.com/",
			"https://{sub}.example_2.com/st_0_0",
			"http://{sub}.example_2.com/st_0_0/{wl_1_0}/{rx_2_0:p0}/st_3_0",
			"http://{sub}.example_2.com/st_0_0/{wl_1_0}/{rx_2_1:p1}/{wl_3_0}/",
			// -----
			"/",
			"/st_0_0",
			"/st_0_0/{wl_1_0}/{rx_2_0:p0}/st_3_0",
			"/st_0_0/{wl_1_0}/{rx_2_1:p1}/{wl_3_0}/",
		];

		for case in cases {
			dbg!(case);

			router.resource_mut(case).set_handler_for(_get(handler));
		}

		let cases = [
			"http://example_0.com".to_string(),
			"http://{sub}.example_1.com".to_string(),
			"http://{sub}.example_2.com".to_string(),
			"".to_string(),
		];

		for case in cases {
			let root = router.resource_mut(case.clone() + "/");
			assert!(root.can_handle_request());
			assert!(!root.ends_with_slash());

			let wl_1_0 = router.resource_mut(case.clone() + "/st_0_0/{wl_1_0}");
			assert!(!wl_1_0.can_handle_request());

			let st_0_0 = router.resource_mut(case.clone() + "/st_0_0");
			assert!(st_0_0.can_handle_request());
			assert!(!st_0_0.ends_with_slash());

			// First time we're accessing the rx_2_1. It must be configured to end with a slash.
			let rx_2_1 = router.resource_mut(case.clone() + "/st_0_0/{wl_1_0}/{rx_2_1:p0}/");
			assert!(!rx_2_1.can_handle_request());
			assert!(rx_2_1.ends_with_slash());

			let st_3_0 = router.resource_mut(case.clone() + "/st_0_0/{wl_1_0}/{rx_2_0:p0}/st_3_0");
			assert!(st_3_0.can_handle_request());
			assert!(!st_3_0.ends_with_slash());

			// New resource.
			let st_3_2 = router.resource_mut(case.clone() + "/st_0_0/{wl_1_0}/{rx_2_1:p1}/st_3_2/");
			assert!(!st_3_2.can_handle_request());
			assert!(st_3_2.ends_with_slash());

			let wl_3_0 = router.resource_mut(case.clone() + "/st_0_0/{wl_1_0}/{rx_2_1:p1}/{wl_3_0}/");
			assert!(wl_3_0.can_handle_request());
			assert!(wl_3_0.ends_with_slash());
		}
	}

	#[test]
	fn router_for_each_root() {
		let mut router = Router::new();

		let cases = [
			"http://example_0.com/st_0_0",
			"http://example_0.com/{rx_0_1:p0}/{wl_1_0}/",
			"http://example_0.com/{wl_0_2}/st_1_0",
			// -----
			"http://{sub}.example_1.com/st_0_0",
			"http://{sub}.example_1.com/{rx_0_1:p0}/{wl_1_0}/",
			"http://{sub}.example_1.com/{wl_0_2}/st_1_0",
			// -----
			"http://{sub}.example_2.com/st_0_0",
			"http://{sub}.example_2.com/{rx_0_1:p0}/{wl_1_0}/",
			"http://{sub}.example_2.com/{wl_0_2}/st_1_0",
			// -----
			"/st_0_0",
			"/{rx_0_1:p0}/{wl_1_0}/",
			"/{wl_0_2}/st_1_0",
		];

		for case in cases {
			router.resource_mut(case);
		}

		router.for_each_root((), |_, root| {
			if root.host_is("{sub}.example_1.com") {
				dbg!("{sub}.example_1.com");
				return Iteration::Continue;
			}

			root.configure(_to_drop_on_unmatching_slash());
			root.for_each_subresource((), |_, resource| {
				dbg!(resource.pattern_string());
				resource.configure(_to_drop_on_unmatching_slash());

				if resource.is("{rx_0_1:p0}") {
					Iteration::Skip
				} else {
					Iteration::Continue
				}
			});

			Iteration::Continue
		});

		let cases = [
			"http://example_0.com".to_string(),
			"http://{sub}.example_1.com".to_string(),
			"http://{sub}.example_2.com".to_string(),
			"".to_string(),
		];

		for case in cases {
			if case == "http://{sub}.example_1.com" {
				assert!(!router
					.resource_mut(case.clone() + "/st_0_0")
					.drops_on_unmatching_slash());

				assert!(!router
					.resource_mut(case.clone() + "/{rx_0_1:p0}")
					.drops_on_unmatching_slash());

				assert!(!router
					.resource_mut(case.clone() + "/{rx_0_1:p0}/{wl_1_0}/")
					.drops_on_unmatching_slash());

				assert!(!router
					.resource_mut(case.clone() + "/{wl_0_2}")
					.drops_on_unmatching_slash());

				assert!(!router
					.resource_mut(case + "/{wl_0_2}/st_1_0")
					.drops_on_unmatching_slash());

				continue;
			}

			assert!(router
				.resource_mut(case.clone() + "/st_0_0")
				.drops_on_unmatching_slash());

			assert!(router
				.resource_mut(case.clone() + "/{rx_0_1:p0}")
				.drops_on_unmatching_slash());

			assert!(!router
				.resource_mut(case.clone() + "/{rx_0_1:p0}/{wl_1_0}/")
				.drops_on_unmatching_slash());

			assert!(router
				.resource_mut(case.clone() + "/{wl_0_2}")
				.drops_on_unmatching_slash());

			assert!(router
				.resource_mut(case + "/{wl_0_2}/st_1_0")
				.drops_on_unmatching_slash());
		}
	}
}
