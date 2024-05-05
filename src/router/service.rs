use std::{borrow::Cow, convert::Infallible, future::ready, sync::Arc};

use argan_core::{
	body::{Body, HttpBody},
	BoxedError, BoxedFuture,
};
use bytes::Bytes;
use http::{Extensions, StatusCode};
use hyper::service::Service;

use crate::{
	common::{MaybeBoxed, NodeExtensions, Uncloneable, SCOPE_VALIDITY},
	handler::{futures::ResponseToResultFuture, Args, BoxedHandler, Handler},
	host::{Host, HostService},
	middleware::{targets::LayerTarget, Layer},
	pattern::ParamsList,
	request::{ContextProperties, Request, RequestContext},
	resource::{Resource, ResourceService},
	response::{BoxedErrorResponse, InfallibleResponseFuture, IntoResponse, Response},
	routing::{RouteTraversal, RoutingState},
};

use super::Router;

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

/// A router service that can be used to handle requests.
///
/// Created by calling [`Router::into_service()`] on a `Router`.
pub struct RouterService {
	context_properties: ContextProperties,
	extensions: Extensions,
	request_passer: MaybeBoxed<RequestPasser>,
}

impl RouterService {
	#[inline(always)]
	pub(super) fn new(
		context_properties: ContextProperties,
		extensions: Extensions,
		request_passer: MaybeBoxed<RequestPasser>,
	) -> Self {
		Self {
			context_properties,
			extensions,
			request_passer,
		}
	}
}

impl<B> Service<Request<B>> for RouterService
where
	B: HttpBody<Data = Bytes> + Send + Sync + 'static,
	B::Error: Into<BoxedError>,
{
	type Response = Response;
	type Error = Infallible;
	type Future = InfallibleResponseFuture;

	fn call(&self, mut request: Request<B>) -> Self::Future {
		let routing_state = RoutingState::new(RouteTraversal::for_route(request.uri().path()));

		let args = Args {
			node_extensions: NodeExtensions::new_borrowed(&self.extensions),
			handler_extension: Cow::Borrowed(&()),
		};

		let request_context =
			RequestContext::new(request, routing_state, self.context_properties.clone());

		match &self.request_passer {
			MaybeBoxed::Boxed(boxed_request_passer) => InfallibleResponseFuture::from(
				boxed_request_passer.handle(request_context.map(Body::new), args),
			),
			MaybeBoxed::Unboxed(request_passer) => {
				InfallibleResponseFuture::from(request_passer.handle(request_context, args))
			}
		}
	}
}

// -------------------------

/// A router service that uses `Arc`.
///
/// Created by calling [`Router::into_arc_service()`] on a `Router`.
pub struct ArcRouterService(Arc<RouterService>);

impl From<RouterService> for ArcRouterService {
	#[inline(always)]
	fn from(router_service: RouterService) -> Self {
		ArcRouterService(Arc::new(router_service))
	}
}

impl Clone for ArcRouterService {
	fn clone(&self) -> Self {
		ArcRouterService(Arc::clone(&self.0))
	}
}

impl<B> Service<Request<B>> for ArcRouterService
where
	B: HttpBody<Data = Bytes> + Send + Sync + 'static,
	B::Error: Into<BoxedError>,
{
	type Response = Response;
	type Error = Infallible;
	type Future = InfallibleResponseFuture;

	#[inline(always)]
	fn call(&self, request: Request<B>) -> Self::Future {
		self.0.as_ref().call(request)
	}
}

// -------------------------

/// A router service that uses leaked `&'static`.
///
/// Created by calling [`Router::into_leaked_service()`] on a `Router`.
#[derive(Clone)]
pub struct LeakedRouterService(&'static RouterService);

impl From<RouterService> for LeakedRouterService {
	#[inline(always)]
	fn from(router_service: RouterService) -> Self {
		let router_service_static_ref = Box::leak(Box::new(router_service));

		LeakedRouterService(router_service_static_ref)
	}
}

impl<B> Service<Request<B>> for LeakedRouterService
where
	B: HttpBody<Data = Bytes> + Send + Sync + 'static,
	B::Error: Into<BoxedError>,
{
	type Response = Response;
	type Error = Infallible;
	type Future = InfallibleResponseFuture;

	#[inline(always)]
	fn call(&self, request: Request<B>) -> Self::Future {
		self.0.call(request)
	}
}

// --------------------------------------------------
// RequestPasser

#[derive(Clone)]
pub(super) struct RequestPasser {
	some_static_hosts: Option<Arc<[HostService]>>,
	some_regex_hosts: Option<Arc<[HostService]>>,
	some_root_resource: Option<Arc<ResourceService>>,
}

impl RequestPasser {
	pub(super) fn new(
		some_static_hosts: Option<Arc<[HostService]>>,
		some_regex_hosts: Option<Arc<[HostService]>>,
		some_root_resource: Option<Arc<ResourceService>>,
		middleware: Vec<LayerTarget<Router>>,
	) -> MaybeBoxed<Self> {
		let request_passer = Self {
			some_static_hosts,
			some_regex_hosts,
			some_root_resource,
		};

		let mut maybe_boxed_request_passer = MaybeBoxed::Unboxed(request_passer);

		for mut layer in middleware.into_iter().rev() {
			match layer {
				LayerTarget::RequestPasser(_) => {
					let LayerTarget::RequestPasser(boxed_layer) = layer.take() else {
						unreachable!()
					};

					match maybe_boxed_request_passer {
						MaybeBoxed::Boxed(boxed_request_passer) => {
							maybe_boxed_request_passer =
								MaybeBoxed::Boxed(boxed_layer.wrap(boxed_request_passer.into()));
						}
						MaybeBoxed::Unboxed(request_passer) => {
							let mut boxed_requst_passer = BoxedHandler::new(request_passer);

							maybe_boxed_request_passer =
								MaybeBoxed::Boxed(boxed_layer.wrap(boxed_requst_passer.into()));
						}
					}
				}
				_ => {}
			}
		}

		maybe_boxed_request_passer
	}
}

impl<B> Handler<B> for RequestPasser
where
	B: HttpBody<Data = Bytes> + Send + Sync + 'static,
	B::Error: Into<BoxedError>,
{
	type Response = Response;
	type Error = BoxedErrorResponse;
	type Future = BoxedFuture<Result<Self::Response, Self::Error>>;

	fn handle(&self, mut request: RequestContext<B>, mut args: Args) -> Self::Future {
		if let Some((uri_host, uri_params)) = request.routing_host_and_uri_params_mut() {
			if let Some(host) = self.some_static_hosts.as_ref().and_then(|hosts| {
				hosts.iter().find(|host| {
					host
						.is_static_match(uri_host)
						.expect("static_hosts must keep only the hosts with a static pattern")
				})
			}) {
				return host.handle(request, args);
			}

			#[cfg(feature = "regex")]
			if let Some(host) = self.some_regex_hosts.as_ref().and_then(|hosts| {
				hosts.iter().find(|host| {
					host
						.is_regex_match(uri_host, uri_params)
						.expect("regex_hosts must keep only the hosts with a static pattern")
				})
			}) {
				return host.handle(request, args);
			}
		}

		if let Some(root_resource) = self.some_root_resource.as_deref() {
			return root_resource.handle(request, args);
		}

		Box::pin(ready(Ok(StatusCode::NOT_FOUND.into_response())))
	}
}

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

#[cfg(all(test, feature = "full"))]
mod test {
	use http_body_util::{BodyExt, Empty};

	use crate::{
		common::{
			config::_with_request_extensions_modifier,
			test_helpers::{new_root, test_service, Case, DataKind, Rx_1_1, Rx_2_0, Wl_3_0},
		},
		handler::_get,
		request::RequestHead,
		router::Router,
	};

	use super::*;

	// --------------------------------------------------------------------------------
	// --------------------------------------------------------------------------------

	#[tokio::test]
	async fn router_service() {
		let cases = [
			// -------------------------
			//	http://example.com
			//
			//	http://example.com/	->	/st_0_0	->	/{wl_1_0}	->	/{rx_2_0:p_0}/
			//										|						|	->	/{rx_2_1:p_1}-abc	->	/{wl_3_0}
			//										|
			//										|	->	/{rx_1_1:p_0}-abc/	->	/st_2_0
			//																						|	->	/st_2_1
			//
			Case {
				name: "root",
				method: "GET",
				host: "http://example.com",
				path: "",
				some_content_type: Some(mime::TEXT_PLAIN_UTF_8),
				some_redirect_location: None,
				data_kind: DataKind::String("Hello, World!".to_string()),
			},
			Case {
				name: "st_0_0",
				method: "GET",
				host: "http://example.com",
				path: "/st_0_0",
				some_content_type: None,
				some_redirect_location: None,
				data_kind: DataKind::None,
			},
			Case {
				name: "rx_2_0",
				method: "GET",
				host: "http://example.com",
				path: "/st_0_0/42/p_0",
				some_content_type: None,
				some_redirect_location: Some("/st_0_0/42/p_0/"),
				data_kind: DataKind::None,
			},
			Case {
				name: "rx_2_0",
				method: "GET",
				host: "http://example.com",
				path: "/st_0_0/42/p_0/",
				some_content_type: Some(mime::APPLICATION_JSON),
				some_redirect_location: None,
				data_kind: DataKind::Rx_2_0(Rx_2_0 {
					sub: None,
					wl_1_0: 42,
					rx_2_0: "p_0".to_string(),
				}),
			},
			Case {
				name: "wl_3_0",
				method: "POST",
				host: "http://example.com",
				path: "/st_0_0/42/p_1-abc/true/",
				some_content_type: None,
				some_redirect_location: Some("/st_0_0/42/p_1-abc/true"),
				data_kind: DataKind::None,
			},
			Case {
				name: "wl_3_0",
				method: "POST",
				host: "http://example.com",
				path: "/st_0_0/42/p_1-abc/true",
				some_content_type: Some(mime::APPLICATION_JSON),
				some_redirect_location: None,
				data_kind: DataKind::Wl_3_0(Wl_3_0 {
					sub: None,
					wl_1_0: 42,
					rx_2_1: "p_1".to_string(),
					wl_3_0: true,
				}),
			},
			Case {
				name: "st_2_0",
				method: "GET",
				host: "http://example.com",
				path: "/st_0_0/p_0-abc/st_2_0",
				some_content_type: Some(mime::APPLICATION_JSON),
				some_redirect_location: None,
				data_kind: DataKind::Rx_1_1(Rx_1_1 {
					sub: None,
					rx_1_1: "p_0".to_string(),
				}),
			},
			Case {
				name: "rx_1_1",
				method: "GET",
				host: "http://example.com",
				path: "/st_0_0/p_0-abc",
				some_content_type: None,
				some_redirect_location: Some("/st_0_0/p_0-abc/"),
				data_kind: DataKind::None,
			},
			Case {
				name: "rx_1_1",
				method: "PUT",
				host: "http://example.com",
				path: "/st_0_0/p_0-abc/",
				some_content_type: Some(mime::APPLICATION_JSON),
				some_redirect_location: None,
				data_kind: DataKind::Rx_1_1(Rx_1_1 {
					sub: None,
					rx_1_1: "p_0".to_string(),
				}),
			},
			Case {
				name: "st_2_1",
				method: "GET",
				host: "http://example.com",
				path: "/st_0_0/p_0-abc/st_2_1",
				some_content_type: Some(mime::TEXT_PLAIN_UTF_8),
				some_redirect_location: None,
				data_kind: DataKind::String("Hello, World!".to_string()),
			},
			//
			// -------------------------
			//	http://{sub}.example.com
			//
			//	http://{sub}.example.com/	->	/st_0_0	->	/{wl_1_0}	->	/{rx_2_0:p_0}/
			//													|						|	->	/{rx_2_1:p_1}-abc	->	/{wl_3_0}
			//													|
			//													|	->	/{rx_1_1:p_0}-abc/	->	/st_2_0
			//																									|	->	/st_2_1
			//
			Case {
				name: "root",
				method: "GET",
				host: "http://www.example.com",
				path: "",
				some_content_type: Some(mime::APPLICATION_JSON),
				some_redirect_location: None,
				data_kind: DataKind::String("www".to_string()),
			},
			Case {
				name: "st_0_0",
				method: "GET",
				host: "http://www.example.com",
				path: "/st_0_0",
				some_content_type: None,
				some_redirect_location: None,
				data_kind: DataKind::None,
			},
			Case {
				name: "rx_2_0",
				method: "GET",
				host: "http://www.example.com",
				path: "/st_0_0/42/p_0",
				some_content_type: None,
				some_redirect_location: Some("/st_0_0/42/p_0/"),
				data_kind: DataKind::None,
			},
			Case {
				name: "rx_2_0",
				method: "GET",
				host: "http://www.example.com",
				path: "/st_0_0/42/p_0/",
				some_content_type: Some(mime::APPLICATION_JSON),
				some_redirect_location: None,
				data_kind: DataKind::Rx_2_0(Rx_2_0 {
					sub: Some("www".to_string()),
					wl_1_0: 42,
					rx_2_0: "p_0".to_string(),
				}),
			},
			Case {
				name: "wl_3_0",
				method: "POST",
				host: "http://www.example.com",
				path: "/st_0_0/42/p_1-abc/true/",
				some_content_type: None,
				some_redirect_location: Some("/st_0_0/42/p_1-abc/true"),
				data_kind: DataKind::None,
			},
			Case {
				name: "wl_3_0",
				method: "POST",
				host: "http://www.example.com",
				path: "/st_0_0/42/p_1-abc/true",
				some_content_type: Some(mime::APPLICATION_JSON),
				some_redirect_location: None,
				data_kind: DataKind::Wl_3_0(Wl_3_0 {
					sub: Some("www".to_string()),
					wl_1_0: 42,
					rx_2_1: "p_1".to_string(),
					wl_3_0: true,
				}),
			},
			Case {
				name: "st_2_0",
				method: "GET",
				host: "http://www.example.com",
				path: "/st_0_0/p_0-abc/st_2_0",
				some_content_type: Some(mime::APPLICATION_JSON),
				some_redirect_location: None,
				data_kind: DataKind::Rx_1_1(Rx_1_1 {
					sub: Some("www".to_string()),
					rx_1_1: "p_0".to_string(),
				}),
			},
			Case {
				name: "rx_1_1",
				method: "GET",
				host: "http://www.example.com",
				path: "/st_0_0/p_0-abc",
				some_content_type: None,
				some_redirect_location: Some("/st_0_0/p_0-abc/"),
				data_kind: DataKind::None,
			},
			Case {
				name: "rx_1_1",
				method: "PUT",
				host: "http://www.example.com",
				path: "/st_0_0/p_0-abc/",
				some_content_type: Some(mime::APPLICATION_JSON),
				some_redirect_location: None,
				data_kind: DataKind::Rx_1_1(Rx_1_1 {
					sub: Some("www".to_string()),
					rx_1_1: "p_0".to_string(),
				}),
			},
			Case {
				name: "st_2_1",
				method: "GET",
				host: "http://www.example.com",
				path: "/st_0_0/p_0-abc/st_2_1",
				some_content_type: Some(mime::TEXT_PLAIN_UTF_8),
				some_redirect_location: None,
				data_kind: DataKind::String("Hello, World!".to_string()),
			},
			//
			// -------------------------
			//	root
			//
			//	/	->	/st_0_0	->	/{wl_1_0}	->	/{rx_2_0:p_0}/
			//							|							|	->	/{rx_2_1:p_1}-abc	->	/{wl_3_0}
			//							|
			//							|	->	/{rx_1_1:p_0}-abc/	->	/st_2_0
			//																			|	->	/st_2_1
			//
			Case {
				name: "root",
				method: "GET",
				host: "",
				path: "/",
				some_content_type: Some(mime::TEXT_PLAIN_UTF_8),
				some_redirect_location: None,
				data_kind: DataKind::String("Hello, World!".to_string()),
			},
			Case {
				name: "st_0_0",
				method: "GET",
				host: "",
				path: "/st_0_0",
				some_content_type: None,
				some_redirect_location: None,
				data_kind: DataKind::None,
			},
			Case {
				name: "rx_2_0",
				method: "GET",
				host: "",
				path: "/st_0_0/42/p_0",
				some_content_type: None,
				some_redirect_location: Some("/st_0_0/42/p_0/"),
				data_kind: DataKind::None,
			},
			Case {
				name: "rx_2_0",
				method: "GET",
				host: "",
				path: "/st_0_0/42/p_0/",
				some_content_type: Some(mime::APPLICATION_JSON),
				some_redirect_location: None,
				data_kind: DataKind::Rx_2_0(Rx_2_0 {
					sub: None,
					wl_1_0: 42,
					rx_2_0: "p_0".to_string(),
				}),
			},
			Case {
				name: "wl_3_0",
				method: "POST",
				host: "",
				path: "/st_0_0/42/p_1-abc/true/",
				some_content_type: None,
				some_redirect_location: Some("/st_0_0/42/p_1-abc/true"),
				data_kind: DataKind::None,
			},
			Case {
				name: "wl_3_0",
				method: "POST",
				host: "",
				path: "/st_0_0/42/p_1-abc/true",
				some_content_type: Some(mime::APPLICATION_JSON),
				some_redirect_location: None,
				data_kind: DataKind::Wl_3_0(Wl_3_0 {
					sub: None,
					wl_1_0: 42,
					rx_2_1: "p_1".to_string(),
					wl_3_0: true,
				}),
			},
			Case {
				name: "st_2_0",
				method: "GET",
				host: "",
				path: "/st_0_0/p_0-abc/st_2_0",
				some_content_type: Some(mime::APPLICATION_JSON),
				some_redirect_location: None,
				data_kind: DataKind::Rx_1_1(Rx_1_1 {
					sub: None,
					rx_1_1: "p_0".to_string(),
				}),
			},
			Case {
				name: "rx_1_1",
				method: "GET",
				host: "",
				path: "/st_0_0/p_0-abc",
				some_content_type: None,
				some_redirect_location: Some("/st_0_0/p_0-abc/"),
				data_kind: DataKind::None,
			},
			Case {
				name: "rx_1_1",
				method: "PUT",
				host: "",
				path: "/st_0_0/p_0-abc/",
				some_content_type: Some(mime::APPLICATION_JSON),
				some_redirect_location: None,
				data_kind: DataKind::Rx_1_1(Rx_1_1 {
					sub: None,
					rx_1_1: "p_0".to_string(),
				}),
			},
			Case {
				name: "st_2_1",
				method: "GET",
				host: "",
				path: "/st_0_0/p_0-abc/st_2_1",
				some_content_type: Some(mime::TEXT_PLAIN_UTF_8),
				some_redirect_location: None,
				data_kind: DataKind::String("Hello, World!".to_string()),
			},
		];

		let mut router = Router::new();
		router.add_resource_under("http://example.com", new_root());
		router.add_resource_under("http://{sub}.example.com", new_root());
		router.add_resource(new_root());

		let service = router.into_service();

		test_service(service, &cases).await;
	}

	#[tokio::test]
	async fn router_request_extensions() {
		let mut router = Router::new();
		router.configure(_with_request_extensions_modifier(|extensions| {
			extensions.insert("Hello from Handler!".to_string());
		}));

		router
			.resource_mut("/st_0_0/st_1_0")
			.set_handler_for(_get(|head: RequestHead| async move {
				head.extensions_ref().get::<String>().unwrap().clone()
			}));

		// ----------

		let service = router.into_service();

		let request = Request::builder()
			.uri("/st_0_0/st_1_0")
			.body(Empty::default())
			.unwrap();

		let response = service.call(request).await.unwrap();
		assert_eq!(response.status(), StatusCode::OK);

		let body = response.collect().await.unwrap().to_bytes();
		assert_eq!(body, "Hello from Handler!");
	}
}
