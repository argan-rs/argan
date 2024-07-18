use std::{convert::Infallible, future::ready, net::SocketAddr, sync::Arc};

use argan_core::{
	body::{Body, HttpBody},
	response::ErrorResponse,
	BoxedError, BoxedFuture,
};
use bytes::Bytes;
use http::StatusCode;
use hyper::service::Service;

use crate::{
	common::{
		header_utils::HostHeaderError, marker::Sealed, CloneWithPeerAddr, MaybeBoxed, NodeExtension,
	},
	handler::{Args, BoxedHandler, Handler},
	host::FinalHost,
	middleware::{targets::LayerTarget, Layer},
	request::{
		routing::{RouteTraversal, RoutingState},
		Request, RequestContext, RequestContextProperties,
	},
	resource::FinalResource,
	response::{BoxedErrorResponse, InfallibleResponseFuture, IntoResponse, Response},
};

#[cfg(feature = "peer-addr")]
use crate::common::SCOPE_VALIDITY;

use super::Router;

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

#[derive(Clone)]
pub(super) struct FinalRouter {
	request_context_properties: RequestContextProperties,
	extension: NodeExtension,
	request_passer: MaybeBoxed<RouterRequestPasser>,
}

impl FinalRouter {
	#[inline(always)]
	pub(super) fn new(
		request_context_properties: RequestContextProperties,
		extension: NodeExtension,
		request_passer: MaybeBoxed<RouterRequestPasser>,
	) -> Self {
		Self {
			request_context_properties,
			extension,
			request_passer,
		}
	}
}

impl AsRef<FinalRouter> for FinalRouter {
	#[inline(always)]
	fn as_ref(&self) -> &FinalRouter {
		self
	}
}

// --------------------------------------------------
// InnerRouterService

struct InnerRouterService<R = FinalRouter> {
	router: R,

	#[cfg(feature = "peer-addr")]
	peer_addr: SocketAddr,
}

impl<R: AsRef<FinalRouter>> InnerRouterService<R> {
	#[inline(always)]
	fn new(final_router: R) -> Self {
		Self {
			router: final_router,

			#[cfg(feature = "peer-addr")]
			peer_addr: "0.0.0.0:0".parse().expect(SCOPE_VALIDITY),
		}
	}

	#[inline(always)]
	fn router_ref(&self) -> &FinalRouter {
		self.router.as_ref()
	}
}

// ----------

impl Clone for InnerRouterService<FinalRouter> {
	#[inline(always)]
	fn clone(&self) -> Self {
		Self {
			router: self.router.clone(),

			#[cfg(feature = "peer-addr")]
			peer_addr: self.peer_addr,
		}
	}
}

impl Clone for InnerRouterService<Arc<FinalRouter>> {
	#[inline(always)]
	fn clone(&self) -> Self {
		Self {
			router: Arc::clone(&self.router),

			#[cfg(feature = "peer-addr")]
			peer_addr: self.peer_addr,
		}
	}
}

impl Clone for InnerRouterService<&'static FinalRouter> {
	#[inline(always)]
	fn clone(&self) -> Self {
		Self {
			router: self.router,

			#[cfg(feature = "peer-addr")]
			peer_addr: self.peer_addr,
		}
	}
}

// ----------

impl<B, R> Service<Request<B>> for InnerRouterService<R>
where
	B: HttpBody<Data = Bytes> + Send + Sync + 'static,
	B::Error: Into<BoxedError>,
	R: AsRef<FinalRouter>,
{
	type Response = Response;
	type Error = Infallible;
	type Future = InfallibleResponseFuture;

	fn call(&self, request: Request<B>) -> Self::Future {
		let routing_state = RoutingState::new(RouteTraversal::for_route(request.uri().path()));

		let args = Args::new_with_node_extension_ref(&self.router_ref().extension);

		let request_context = RequestContext::new(
			#[cfg(feature = "peer-addr")]
			self.peer_addr,
			request,
			routing_state,
			self.router_ref().request_context_properties.clone(),
		);

		match &self.router_ref().request_passer {
			MaybeBoxed::Boxed(boxed_request_passer) => InfallibleResponseFuture::from(
				boxed_request_passer.handle(request_context.map(Body::new), args),
			),
			MaybeBoxed::Unboxed(request_passer) => {
				InfallibleResponseFuture::from(request_passer.handle(request_context, args))
			}
		}
	}
}

// --------------------------------------------------
// RouterService

/// A router service that can be used to handle requests.
///
/// Created by calling [`Router::into_service()`] on a `Router`.
#[derive(Clone)]
pub struct RouterService(InnerRouterService<FinalRouter>);

impl RouterService {
	#[inline(always)]
	pub(super) fn new(final_router: FinalRouter) -> Self {
		Self(InnerRouterService::new(final_router))
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

	#[inline(always)]
	fn call(&self, request: Request<B>) -> Self::Future {
		self.0.call(request)
	}
}

impl CloneWithPeerAddr for RouterService {
	#[inline(always)]
	fn clone_with_peer_addr(&self, _addr: SocketAddr) -> Self {
		Self(InnerRouterService {
			router: self.0.router.clone(),
			#[cfg(feature = "peer-addr")]
			peer_addr: _addr,
		})
	}
}

impl Sealed for RouterService {}

// --------------------------------------------------
// ArcRouterService

/// A router service that uses `Arc`.
///
/// Created by calling [`Router::into_arc_service()`] on a `Router`.
#[derive(Clone)]
pub struct ArcRouterService(InnerRouterService<Arc<FinalRouter>>);

impl From<RouterService> for ArcRouterService {
	#[inline(always)]
	fn from(router_service: RouterService) -> Self {
		let RouterService(InnerRouterService {
			router,

			#[cfg(feature = "peer-addr")]
			peer_addr,
		}) = router_service;

		Self(InnerRouterService {
			router: Arc::new(router),

			#[cfg(feature = "peer-addr")]
			peer_addr,
		})
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
		self.0.call(request)
	}
}

impl CloneWithPeerAddr for ArcRouterService {
	fn clone_with_peer_addr(&self, _addr: SocketAddr) -> Self {
		Self(InnerRouterService {
			router: Arc::clone(&self.0.router),
			#[cfg(feature = "peer-addr")]
			peer_addr: _addr,
		})
	}
}

impl Sealed for ArcRouterService {}

// --------------------------------------------------
// LeakedRouterService

/// A router service that uses leaked `&'static`.
///
/// Created by calling [`Router::into_leaked_service()`] on a `Router`.
#[derive(Clone)]
pub struct LeakedRouterService(InnerRouterService<&'static FinalRouter>);

impl From<RouterService> for LeakedRouterService {
	#[inline(always)]
	fn from(router_service: RouterService) -> Self {
		let RouterService(InnerRouterService {
			router,

			#[cfg(feature = "peer-addr")]
			peer_addr,
		}) = router_service;

		let final_router_ref = Box::leak(Box::new(router));

		Self(InnerRouterService {
			router: final_router_ref,

			#[cfg(feature = "peer-addr")]
			peer_addr,
		})
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

impl CloneWithPeerAddr for LeakedRouterService {
	fn clone_with_peer_addr(&self, _addr: SocketAddr) -> Self {
		Self(InnerRouterService {
			router: self.0.router,
			#[cfg(feature = "peer-addr")]
			peer_addr: _addr,
		})
	}
}

impl Sealed for LeakedRouterService {}

// --------------------------------------------------
// RequestPasser

#[derive(Clone)]
pub(super) struct RouterRequestPasser {
	some_static_hosts: Option<Arc<[FinalHost]>>,
	some_regex_hosts: Option<Arc<[FinalHost]>>,
	some_root_resource: Option<Arc<FinalResource>>,
}

impl RouterRequestPasser {
	pub(super) fn new(
		some_static_hosts: Option<Arc<[FinalHost]>>,
		some_regex_hosts: Option<Arc<[FinalHost]>>,
		some_root_resource: Option<Arc<FinalResource>>,
		middleware: Vec<LayerTarget<Router>>,
	) -> MaybeBoxed<Self> {
		let request_passer = Self {
			some_static_hosts,
			some_regex_hosts,
			some_root_resource,
		};

		let mut maybe_boxed_request_passer = MaybeBoxed::Unboxed(request_passer);

		for mut layer in middleware.into_iter().rev() {
			if let LayerTarget::RequestPasser(_) = layer {
				let LayerTarget::RequestPasser(boxed_layer) = layer.take() else {
					unreachable!()
				};

				match maybe_boxed_request_passer {
					MaybeBoxed::Boxed(boxed_request_passer) => {
						maybe_boxed_request_passer =
							MaybeBoxed::Boxed(boxed_layer.wrap(boxed_request_passer.into()));
					}
					MaybeBoxed::Unboxed(request_passer) => {
						let boxed_requst_passer = BoxedHandler::new(request_passer);

						maybe_boxed_request_passer =
							MaybeBoxed::Boxed(boxed_layer.wrap(boxed_requst_passer.into()));
					}
				}
			}
		}

		maybe_boxed_request_passer
	}
}

impl<B> Handler<B> for RouterRequestPasser
where
	B: HttpBody<Data = Bytes> + Send + Sync + 'static,
	B::Error: Into<BoxedError>,
{
	type Response = Response;
	type Error = BoxedErrorResponse;
	type Future = BoxedFuture<Result<Self::Response, Self::Error>>;

	fn handle(&self, mut request: RequestContext<B>, args: Args) -> Self::Future {
		#[allow(unused_variables)]
		match request.routing_host_and_uri_params_mut() {
			Ok((uri_host, uri_params)) => {
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
			Err(HostHeaderError::Missing) => {}
			Err(error) => return Box::pin(ready(error.into_error_result())),
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
	use http::{
		header::{HOST, LOCATION},
		Extensions, Method,
	};
	use http_body_util::{BodyExt, Empty};

	use crate::{
		common::test_helpers::{new_root, test_service, Case, DataKind, Rx_1_1, Rx_2_0, Wl_3_0},
		handler::HandlerSetter,
		middleware::{
			RedirectionLayer, RequestExtensionsModifierLayer, RequestPasser, RequestReceiver,
		},
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
		router.wrap(
			RequestPasser.component_in(RequestExtensionsModifierLayer::new(
				|extensions: &mut Extensions| {
					extensions.insert("Hello from Handler!".to_string());
				},
			)),
		);

		router
			.resource_mut("/st_0_0/st_1_0")
			.set_handler_for(Method::GET.to(|head: RequestHead| async move {
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

	#[tokio::test]
	async fn router_host_redirection() {
		let mut router = Router::new();
		let _ = router.resource_mut("http://www.example.com/");

		let example_com_root = router.resource_mut("http://example.com/");
		example_com_root.wrap(RequestReceiver.component_in(
			RedirectionLayer::for_permanent_redirection_to("http://www.example.com/resource"),
		));

		// ----------

		let service = router.into_service();

		let request = Request::builder()
			.uri("http://example.com")
			.body(Empty::default())
			.unwrap();

		let response = service.call(request).await.unwrap();
		assert_eq!(response.status(), StatusCode::PERMANENT_REDIRECT);

		let location = response.headers().get(LOCATION).unwrap();
		assert_eq!(
			"http://www.example.com/resource",
			location.to_str().unwrap()
		);
	}

	#[tokio::test]
	async fn router_host_prefix_redirection() {
		let mut router = Router::new();
		let _ = router.resource_mut("http://www.example.com/");

		let example_com_root = router.resource_mut("http://example.com/");
		example_com_root.wrap(RequestReceiver.component_in(
			RedirectionLayer::for_permanent_redirection_to_prefix("http://www.example.com"),
		));

		// ----------

		let service = router.into_service();

		let request = Request::builder()
			.uri("/resource")
			.header(HOST, "example.com")
			.body(Empty::default())
			.unwrap();

		let response = service.call(request).await.unwrap();
		assert_eq!(response.status(), StatusCode::PERMANENT_REDIRECT);

		let location = response.headers().get(LOCATION).unwrap();
		assert_eq!(
			"http://www.example.com/resource",
			location.to_str().unwrap()
		);
	}
}
