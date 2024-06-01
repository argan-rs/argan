//! Router service types.

// ----------

use core::panic;
use std::{any, sync::Arc};

use http::Extensions;

use crate::{
	common::{node_properties::NodeProperty, IntoArray, SCOPE_VALIDITY},
	host::Host,
	middleware::{targets::LayerTarget, RequestPasser},
	pattern::{split_uri_host_and_path, Pattern, Similarity},
	request::ContextProperties,
	resource::{Iteration, Resource},
};

// --------------------------------------------------

mod service;

pub use service::{ArcRouterService, LeakedRouterService, RouterService};

use self::service::RouterRequestPasser;

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

/// A type that can contain hosts and a root resource.
///
/// The `Router` passes the request to a host that matches the request's 'Host' or
/// to a root resource if one exists when there is no matching host. Otherwise, it
/// responds with `404 Not Found`.
///
///
pub struct Router {
	static_hosts: Vec<Host>,
	regex_hosts: Vec<Host>,
	some_root_resource: Option<Box<Resource>>,

	context_properties: ContextProperties,
	extensions: Extensions,
	middleware: Vec<LayerTarget<Self>>,
}

impl Default for Router {
	fn default() -> Self {
		Self::new()
	}
}

impl Router {
	/// Creates a new `Router`.
	pub fn new() -> Router {
		Self {
			static_hosts: Vec::new(),
			regex_hosts: Vec::new(),
			some_root_resource: None,

			context_properties: ContextProperties::default(),
			extensions: Extensions::new(),
			middleware: Vec::new(),
		}
	}

	/// Adds the given host(s).
	///
	/// ```
	/// use argan::{Router, Host, Resource};
	///
	/// let mut host = Host::new("http://example.com", Resource::new("/"));
	/// let mut host_with_sub = Host::new("http://abc.example.com", Resource::new("/"));
	///
	/// let mut router = Router::new();
	/// router.add_host([host, host_with_sub]);
	/// ```
	///
	/// If a new host has a duplicate among the existing hosts, their resource trees will
	/// be merged. See also the [panics](#panics) below.
	///
	/// ```
	/// use argan::{Router, Host, Resource, handler::HandlerSetter, http::Method};
	///
	/// let mut router = Router::new();
	///
	/// let mut root = Resource::new("/");
	/// root
	///   .subresource_mut("/resource_1/resource_2/resource_3")
	///   .set_handler_for(Method::GET.to(|| async {}));
	///
	/// router.add_host(Host::new("example.com", root));
	///
	/// let mut root = Resource::new("/");
	/// root.subresource_mut("/resource_1/resource_2")
	///   .set_handler_for(Method::GET.to(|| async {}));
	///
	/// router.add_host(Host::new("example.com", root));
	/// ```
	///
	/// # Panics
	///
	/// - if a new host has a duplicate among the existing hosts and both of them have some
	/// resource with the same path and both of those resources have some handler set or
	/// a middleware applied
	///
	/// ```should_panic
	/// use argan::{Router, Host, Resource, handler::HandlerSetter, http::Method};
	///
	/// let mut router = Router::new();
	///
	/// let mut root = Resource::new("/");
	/// root
	///   .subresource_mut("/resource_1/resource_2/resource_3")
	///   .set_handler_for(Method::GET.to(|| async {}));
	///
	/// router.add_host(Host::new("example.com", root));
	///
	/// let mut root = Resource::new("/");
	/// let mut resource_2 = root.subresource_mut("/resource_1/resource_2");
	/// resource_2.set_handler_for(Method::GET.to(|| async {}));
	///
	/// resource_2
	///   .subresource_mut("/resource_3")
	///   .set_handler_for(Method::GET.to(|| async {}));
	///
	/// // This doesn't try to merge the handler sets of the duplicate resources.
	/// router.add_host(Host::new("example.com", root));
	/// ```
	pub fn add_host<H, const N: usize>(&mut self, new_hosts: H)
	where
		H: IntoArray<Host, N>,
	{
		let new_hosts = new_hosts.into_array();

		for new_hosts in new_hosts {
			self.add_single_host(new_hosts)
		}
	}

	fn add_single_host(&mut self, new_host: Host) {
		let (pattern, root) = new_host.into_pattern_and_root();

		if let Some(host) = self.existing_host_mut(&pattern) {
			host.merge_or_replace_root(root);

			return;
		}

		self.add_new_host(pattern, root);
	}

	/// Adds the given resource(s).
	///
	/// ```
	/// use argan::{Router, Resource};
	///
	/// let host_resource = Resource::new("http://example.com/resource");
	/// let root = Resource::new("/");
	///
	/// let mut router = Router::new();
	/// router.add_resource([host_resource, root]);
	/// ```
	///
	/// In the above example, `Router` will have a host with the pattern *"example.com"*
	/// and a root resource.
	///
	/// # Panics
	///
	/// - if the resource or one of its subresources has a duplicate in the existing parent's
	/// subtree and both of them have some handler set or a middleware applied
	///
	/// ```should_panic
	/// use argan::{Router, Resource, handler::HandlerSetter, http::Method};
	///
	/// let mut router = Router::new();
	///
	/// let mut resource_3 = Resource::new("/resource_1/resource_2/resource_3");
	/// resource_3.set_handler_for(Method::GET.to(|| async {}));
	///
	/// router.add_resource(resource_3);
	///
	/// let mut resource_2 = Resource::new("/resource_1/resource_2");
	/// let mut resource_3 = Resource::new("/resource_3");
	/// resource_3.set_handler_for(Method::POST.to(|| async {}));
	///
	/// resource_2.add_subresource(resource_3);
	///
	/// // This doesn't try to merge the handler sets of the duplicate resources.
	/// router.add_resource(resource_2);
	/// ```
	pub fn add_resource<R, const N: usize>(&mut self, new_resources: R)
	where
		R: IntoArray<Resource, N>,
	{
		let new_resources = new_resources.into_array();

		for new_resource in new_resources {
			self.add_single_resource(new_resource)
		}
	}

	fn add_single_resource(&mut self, new_resource: Resource) {
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

		if let Some(host_pattern) = new_resource.host_pattern_ref().cloned() {
			let root = if new_resource.is("/") {
				new_resource
			} else {
				let mut root = Resource::new("/");
				root.set_host_pattern(host_pattern.clone());

				root.add_subresource(new_resource);

				root
			};

			self.add_new_host(host_pattern, root);

			return;
		}

		if new_resource.is("/") {
			self.merge_or_replace_root(new_resource);
		} else if let Some(boxed_root) = self.some_root_resource.as_mut() {
			boxed_root.add_subresource(new_resource);
		} else {
			let mut root = Resource::with_pattern(Pattern::parse("/"));
			root.add_subresource(new_resource);

			self.some_root_resource = Some(Box::new(root));
		}
	}

	fn existing_host_mut(&mut self, host_pattern: &Pattern) -> Option<&mut Host> {
		match host_pattern {
			Pattern::Static(_) => self
				.static_hosts
				.iter_mut()
				.find(|static_host| static_host.compare_pattern(host_pattern) == Similarity::Same),
			#[cfg(feature = "regex")]
			Pattern::Regex(_, _) => self
				.regex_hosts
				.iter_mut()
				.find(|regex_host| regex_host.compare_pattern(host_pattern) == Similarity::Same),
			Pattern::Wildcard(_) => unreachable!(),
		}
	}

	fn add_new_host(&mut self, host_pattern: Pattern, root: Resource) {
		let host = match host_pattern {
			Pattern::Static(_) => &mut self.static_hosts,
			#[cfg(feature = "regex")]
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

	/// Adds the given resources under the prefix URI components.
	///
	/// ```
	/// use argan::{Router, Resource};
	///
	/// let resource_2_0 = Resource::new("/resource_2_0");
	/// let resource_2_1 = Resource::new("/resource_2_1");
	///
	/// let mut router = Router::new();
	/// router.add_resource_under("http://example.com/resource_1", [resource_2_0, resource_2_1]);
	/// ```
	///
	/// # Panics
	///
	/// - if the new resource's URI components don't match the given prefix URI components
	///
	/// ```should_panic
	/// use argan::{Router, Resource};
	///
	/// let mut router = Router::new();
	///
	/// let resource_3 = Resource::new("/resource_1/resource_2/resource_3");
	///
	/// router.add_resource_under("/some_resource", resource_3);
	/// ```
	///
	/// Other **panic** conditions are the same as [add_resource()](Self::add_resource())'s
	/// conditions.
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
		let some_host_pattern = some_host_pattern_str.map(Pattern::parse);

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
				let mut root = Resource::new("/");
				root.set_host_pattern(host_pattern.clone());

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
				let root = Resource::with_pattern(Pattern::parse("/"));
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

	/// Returns the resource at the given URI. If the resource doesn't exist, it
	/// will be created.
	///
	/// ```
	/// use argan::Router;
	///
	/// let mut router = Router::new();
	/// let resource_2 = router.resource_mut("http://example.com/resource_1/resource_2");
	/// ```
	///
	/// # Panics
	/// - if the given URI is empty
	/// - if the URI contains only a path and it doesn't start with a slash `/`
	/// - if the resource has some handler set or middleware applied, and the given
	///   configuration symbols don't match its configuration
	///
	/// ```should_panic
	/// use argan::{Router, handler::HandlerSetter};
	/// use argan::http::Method;
	///
	/// let mut router = Router::new();
	/// router.resource_mut("/resource_1 !*").set_handler_for([
	///   Method::GET.to(|| async {}),
	///   Method::POST.to(|| async {}),
	/// ]);
	///
	/// // ...
	///
	/// let resource_1 = router.resource_mut("/resource_1");
	/// ```
	///
	/// For configuration symbols, see the [`crate documentation`](crate);
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
					#[cfg(feature = "regex")]
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

	/// Adds the given extension to the router. Added extensions are available to all the
	/// middleware that wrap the router's request passer via the [`Args`](crate::handler::Args)
	/// field [`NodeExtensions`](crate::common::NodeExtensions).
	///
	/// # Panics
	///
	/// - if an extension of the same type already exists
	pub fn add_extension<E: Clone + Send + Sync + 'static>(&mut self, extension: E) {
		if self.extensions.insert(extension).is_some() {
			panic!(
				"router already has an extension of type '{}'",
				any::type_name::<E>()
			);
		}
	}

	/// Adds middleware to be applied on the router's request passer.
	///
	/// Middlewares are applied when the router is being converted into a service.
	///
	/// ```
	/// // use declarations
	/// # use std::future::{Future, ready};
	/// # use tower_http::compression::CompressionLayer;
	/// # use argan::{
	/// #   handler::{Handler, Args},
	/// #   middleware::{Layer, RequestPasser},
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
	/// // ...
	///
	/// use argan::Router;
	///
	/// let mut router = Router::new();
	///
	/// router.wrap(RequestPasser.with((CompressionLayer::new(), MiddlewareLayer)));
	/// ```
	pub fn wrap<L, const N: usize>(&mut self, layer_targets: L)
	where
		L: IntoArray<LayerTarget<Self>, N>,
	{
		self.middleware.extend(layer_targets.into_array());
	}

	/// Sets the router's optional properties.
	///
	/// ```
	/// use argan::{Router, common::node_properties::NodeCookieKey, data::cookies::Key};
	///
	/// let mut router = Router::new();
	///
	/// // Given `cookie::Key` will be available to all resoruces unless some resource
	/// // or handler replaces it with its own `cookie::Key` while the request is being
	/// // routed or handled.
	/// router.set_property(NodeCookieKey.to(Key::generate()));
	/// ```
	pub fn set_property<C, const N: usize>(&mut self, properties: C)
	where
		C: IntoArray<NodeProperty<Self>, N>,
	{
		let properties = properties.into_array();

		for property in properties {
			use NodeProperty::*;

			match property {
				#[cfg(any(feature = "private-cookies", feature = "signed-cookies"))]
				CookieKey(cookie_key) => self.context_properties.set_cookie_key(cookie_key),
				RequestExtensionsModifier(request_extensions_modifier_layer) => {
					let request_passer_layer_target = RequestPasser.with(request_extensions_modifier_layer);

					self.middleware.insert(0, request_passer_layer_target);
				}
				_ => unreachable!("ConfigOption::None should never be used"),
			}
		}
	}

	/// Calls the given function for each root resource (hosts' and router's) with a mutable
	/// reference to the `param`.
	///
	/// All the variants of `Iteration` other than `Stop` are ignored. If the function retuns
	/// `Iteration::Stop` or all the root resources have beeen processed, the parameter is
	/// returned in its final state.
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

			if let Iteration::Stop = func(&mut param, root) {
				break param;
			}
		}
	}

	/// Converts the `Router` into a service.
	pub fn into_service(self) -> RouterService {
		let Router {
			static_hosts,
			regex_hosts,
			some_root_resource,
			context_properties: context,
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

		let request_passer = RouterRequestPasser::new(
			some_static_hosts,
			some_regex_hosts,
			some_root_resource,
			middleware,
		);

		RouterService::new(context, extensions, request_passer)
	}

	/// Converts the `Router` into a service that uses `Arc` internally.
	#[inline(always)]
	pub fn into_arc_service(self) -> ArcRouterService {
		ArcRouterService::from(self.into_service())
	}

	/// Converts the `Router` into a service with a leaked `&'static`.
	#[inline(always)]
	pub fn into_leaked_service(self) -> LeakedRouterService {
		LeakedRouterService::from(self.into_service())
	}
}

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

#[cfg(all(test, feature = "full"))]
mod test {
	use http::Method;

	use crate::{common::node_properties::RequestExtensionsModifier, handler::HandlerSetter};

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
			new_root.set_handler_for(Method::GET.to(|| async {}));
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
			new_root.set_handler_for(Method::GET.to(|| async {}));
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
			new_root.set_handler_for(Method::GET.to(|| async {}));
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
			new_root.set_handler_for(Method::GET.to(|| async {}));
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

			router
				.resource_mut(case)
				.set_handler_for(Method::GET.to(handler));
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

			root.set_property(RequestExtensionsModifier.to(|_| {}));
			root.for_each_subresource((), |_, resource| {
				dbg!(resource.pattern_string());
				resource.set_property(RequestExtensionsModifier.to(|_| {}));

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
					.has_some_effect());

				assert!(!router
					.resource_mut(case.clone() + "/{rx_0_1:p0}")
					.has_some_effect());

				assert!(!router
					.resource_mut(case.clone() + "/{rx_0_1:p0}/{wl_1_0}/")
					.has_some_effect());

				assert!(!router
					.resource_mut(case.clone() + "/{wl_0_2}")
					.has_some_effect());

				assert!(!router
					.resource_mut(case + "/{wl_0_2}/st_1_0")
					.has_some_effect());

				continue;
			}

			assert!(router
				.resource_mut(case.clone() + "/st_0_0")
				.has_some_effect());

			assert!(router
				.resource_mut(case.clone() + "/{rx_0_1:p0}")
				.has_some_effect());

			assert!(!router
				.resource_mut(case.clone() + "/{rx_0_1:p0}/{wl_1_0}/")
				.has_some_effect());

			assert!(router
				.resource_mut(case.clone() + "/{wl_0_2}")
				.has_some_effect());

			assert!(router
				.resource_mut(case + "/{wl_0_2}/st_1_0")
				.has_some_effect());
		}
	}
}

// --------------------------------------------------------------------------------
