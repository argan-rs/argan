use std::{borrow::Cow, convert::Infallible, future::ready, sync::Arc};

use argan_core::{
	body::{Body, HttpBody},
	BoxedError, BoxedFuture,
};
use bytes::Bytes;
use http::{Extensions, Method, StatusCode};
use hyper::service::Service;
use percent_encoding::percent_decode_str;

use crate::{
	common::{MaybeBoxed, NodeExtensions, Uncloneable, SCOPE_VALIDITY},
	handler::{
		request_handlers::{handle_mistargeted_request, WildcardMethodHandler},
		ArcHandler, Args, BoxedHandler, Handler,
	},
	middleware::{targets::LayerTarget, BoxedLayer, Layer},
	pattern::{ParamsList, Pattern},
	request::{
		routing::{RouteTraversal, RoutingState},
		ContextProperties, Request, RequestContext,
	},
	response::{BoxedErrorResponse, InfallibleResponseFuture, IntoResponse, Redirect, Response},
};

use super::{config::ConfigFlags, Resource};

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

/// A resource service that can be used to handle requests.
///
/// Created by calling [`Resource::into_service()`] on a `Resource`.
#[derive(Clone)]
pub struct ResourceService {
	pattern: Pattern,
	context_properties: ContextProperties,
	extensions: Extensions,

	request_receiver: MaybeBoxed<RequestReceiver>,
	some_mistargeted_request_handler: Option<ArcHandler>,
}

impl ResourceService {
	#[inline(always)]
	pub(super) fn new(
		pattern: Pattern,
		context_properties: ContextProperties,
		extensions: Extensions,
		request_receiver: MaybeBoxed<RequestReceiver>,
		some_mistargeted_request_handler: Option<ArcHandler>,
	) -> Self {
		Self {
			pattern,
			context_properties,
			extensions,
			request_receiver,
			some_mistargeted_request_handler,
		}
	}

	#[inline(always)]
	fn is_root(&self) -> bool {
		match self.pattern {
			Pattern::Static(ref pattern) => pattern.as_ref() == "/",
			_ => false,
		}
	}

	#[inline(always)]
	pub(crate) fn extensions_ref(&self) -> &Extensions {
		&self.extensions
	}

	#[inline]
	pub(crate) fn handle<B>(
		&self,
		request_context: RequestContext<B>,
		mut args: Args<'_, ()>,
	) -> BoxedFuture<Result<Response, BoxedErrorResponse>>
	where
		B: HttpBody<Data = Bytes> + Send + Sync + 'static,
		B::Error: Into<BoxedError>,
	{
		let mut request_context = request_context.map(Body::new);
		request_context.clone_valid_properties_from(&self.context_properties);

		let args = args.extensions_replaced(NodeExtensions::new_borrowed(&self.extensions), &());

		match &self.request_receiver {
			MaybeBoxed::Boxed(boxed_request_receiver) => {
				boxed_request_receiver.handle(request_context, args)
			}
			MaybeBoxed::Unboxed(request_receiver) => request_receiver.handle(request_context, args),
		}
	}
}

// --------------------------------------------------

impl<B> Service<Request<B>> for ResourceService
where
	B: HttpBody<Data = Bytes> + Send + Sync + 'static,
	B::Error: Into<BoxedError>,
{
	type Response = Response;
	type Error = Infallible;
	type Future = InfallibleResponseFuture;

	fn call(&self, request: Request<B>) -> Self::Future {
		let request = request.map(Body::new);

		let route = request.uri().path();
		let mut route_traversal = RouteTraversal::for_route(route);
		let mut path_params = ParamsList::new();

		let matched = if route == "/" {
			self.is_root()
		} else if self.is_root() {
			// Resource is a root and the request's path always starts from root.
			true
		} else {
			let (next_segment, _) = route_traversal.next_segment(route).expect(SCOPE_VALIDITY);

			// If pattern is static, we may match it without decoding the segment.
			// Static patterns keep percent-encoded string.
			if let Some(result) = self.pattern.is_static_match(next_segment) {
				result
			} else {
				let Ok(decoded_segment) = percent_decode_str(next_segment).decode_utf8() else {
					return InfallibleResponseFuture::from(Box::pin(ready(Ok(
						StatusCode::NOT_FOUND.into_response(),
					))));
				};

				#[cfg(not(feature = "regex"))]
				let some_match_result = None;

				#[cfg(feature = "regex")]
				let some_match_result = self
					.pattern
					.is_regex_match(decoded_segment.as_ref(), &mut path_params);

				if let Some(result) = some_match_result {
					result
				} else {
					self
						.pattern
						.is_wildcard_match(decoded_segment, &mut path_params)
						.expect("wildcard_resource must keep only a resource with a wilcard pattern")
				}
			}
		};

		let mut routing_state = RoutingState::new(route_traversal);
		routing_state.uri_params = path_params;

		let args = Args {
			node_extensions: NodeExtensions::new_borrowed(&self.extensions),
			handler_extension: Cow::Borrowed(&()),
		};

		let request_context =
			RequestContext::new(request, routing_state, self.context_properties.clone());

		if matched {
			match &self.request_receiver {
				MaybeBoxed::Boxed(boxed_request_receiver) => {
					InfallibleResponseFuture::from(boxed_request_receiver.handle(request_context, args))
				}
				MaybeBoxed::Unboxed(request_receiver) => {
					InfallibleResponseFuture::from(request_receiver.handle(request_context, args))
				}
			}
		} else {
			InfallibleResponseFuture::from(handle_mistargeted_request(
				request_context,
				args,
				self.some_mistargeted_request_handler.as_ref(),
			))
		}
	}
}

// -------------------------

/// A resource service that uses `Arc`.
///
/// Created by calling [Resource::into_arc_service()] on a `Resource`.
pub struct ArcResourceService(Arc<ResourceService>);

impl From<ResourceService> for ArcResourceService {
	#[inline(always)]
	fn from(resource_service: ResourceService) -> Self {
		ArcResourceService(Arc::new(resource_service))
	}
}

impl Clone for ArcResourceService {
	fn clone(&self) -> Self {
		ArcResourceService(Arc::clone(&self.0))
	}
}

impl<B> Service<Request<B>> for ArcResourceService
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

/// A resource service that uses leaked `&'static`.
///
/// Created by calling [Resource::into_leaked_service()] on a `Resource`.
#[derive(Clone)]
pub struct LeakedResourceService(&'static ResourceService);

impl From<ResourceService> for LeakedResourceService {
	#[inline(always)]
	fn from(resource_service: ResourceService) -> Self {
		let resource_service_static_ref = Box::leak(Box::new(resource_service));

		LeakedResourceService(resource_service_static_ref)
	}
}

impl<B> Service<Request<B>> for LeakedResourceService
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

#[derive(Clone)]
pub(crate) struct RequestReceiver {
	some_request_passer: Option<MaybeBoxed<RequestPasser>>,
	some_request_handler: Option<Arc<MaybeBoxed<RequestHandler>>>,
	some_mistargeted_request_handler: Option<ArcHandler>,

	config_flags: ConfigFlags,
}

impl RequestReceiver {
	pub(crate) fn new(
		some_request_passer: Option<MaybeBoxed<RequestPasser>>,
		some_request_handler: Option<Arc<MaybeBoxed<RequestHandler>>>,
		some_mistargeted_request_handler: Option<ArcHandler>,
		config_flags: ConfigFlags,
		middleware: Vec<LayerTarget<Resource>>,
	) -> MaybeBoxed<Self> {
		let request_receiver = Self {
			some_request_passer,
			some_request_handler,
			some_mistargeted_request_handler,
			config_flags,
		};

		let mut maybe_boxed_request_receiver = MaybeBoxed::Unboxed(request_receiver);

		for layer in middleware.into_iter().rev() {
			if let LayerTarget::RequestReceiver(boxed_layer) = layer {
				match maybe_boxed_request_receiver {
					MaybeBoxed::Boxed(boxed_request_receiver) => {
						maybe_boxed_request_receiver =
							MaybeBoxed::Boxed(boxed_layer.wrap(boxed_request_receiver.into()));
					}
					MaybeBoxed::Unboxed(request_receiver) => {
						let boxed_request_receiver = BoxedHandler::new(request_receiver);

						maybe_boxed_request_receiver =
							MaybeBoxed::Boxed(boxed_layer.wrap(boxed_request_receiver.into()));
					}
				}
			}
		}

		maybe_boxed_request_receiver
	}

	#[inline(always)]
	fn is_subtree_hander(&self) -> bool {
		self.config_flags.has(ConfigFlags::SUBTREE_HANDLER)
	}
}

impl Handler for RequestReceiver {
	type Response = Response;
	type Error = BoxedErrorResponse;
	type Future = BoxedFuture<Result<Self::Response, Self::Error>>;

	#[inline]
	fn handle(&self, mut request_context: RequestContext, args: Args) -> Self::Future {
		if request_context.routing_has_remaining_segments() {
			let resource_is_subtree_handler = self.is_subtree_hander();

			if let Some(request_passer) = self.some_request_passer.as_ref() {
				if resource_is_subtree_handler {
					request_context.note_subtree_handler();
				}

				let next_segment_index = request_context.routing_next_segment_index();

				let response_future = match request_passer {
					MaybeBoxed::Boxed(boxed_request_passer) => {
						boxed_request_passer.handle(request_context, args)
					}
					MaybeBoxed::Unboxed(request_passer) => request_passer.handle(request_context, args),
				};

				if !resource_is_subtree_handler {
					return response_future;
				}

				let request_handler_clone = self
					.some_request_handler
					.clone()
					.expect("subtree handler must have a request handler");

				return Box::pin(async move {
					let mut response = response_future.await?;
					if response.status() != StatusCode::NOT_FOUND {
						return Ok(response);
					}

					let Some(uncloneable) = response
						.extensions_mut()
						.remove::<Uncloneable<(RequestContext, Args)>>()
					else {
						// Custom 404 Not Found response.
						return Ok(response);
					};

					let (mut request, args) = uncloneable
						.into_inner()
						.expect("unused request and args should always exist in Uncloneable");

					request.routing_revert_to_segment(next_segment_index);

					match request_handler_clone.as_ref() {
						MaybeBoxed::Boxed(boxed_request_handler) => {
							boxed_request_handler.handle(request, args).await
						}
						MaybeBoxed::Unboxed(request_handler) => request_handler.handle(request, args).await,
					}
				});
			}

			if !resource_is_subtree_handler {
				return handle_mistargeted_request(
					request_context,
					args,
					self.some_mistargeted_request_handler.as_ref(),
				);
			}
		}

		if let Some(request_handler) = self.some_request_handler.as_ref() {
			let request_path_ends_with_slash = request_context.path_ends_with_slash();
			let resource_path_ends_with_slash = self.config_flags.has(ConfigFlags::ENDS_WITH_SLASH);

			let handle = if request_path_ends_with_slash && !resource_path_ends_with_slash {
				if self
					.config_flags
					.has(ConfigFlags::REDIRECTS_ON_UNMATCHING_SLASH)
				{
					let path = request_context.uri_ref().path();

					return Box::pin(ready(Ok(
						Redirect::permanently_to(&path[..path.len() - 1]).into_response(),
					)));
				}

				!self
					.config_flags
					.has(ConfigFlags::DROPS_ON_UNMATCHING_SLASH)
			} else if !request_path_ends_with_slash && resource_path_ends_with_slash {
				if self
					.config_flags
					.has(ConfigFlags::REDIRECTS_ON_UNMATCHING_SLASH)
				{
					let path = request_context.uri_ref().path();

					let mut new_path = String::with_capacity(path.len() + 1);
					new_path.push_str(path);
					new_path.push('/');

					return Box::pin(ready(Ok(
						Redirect::permanently_to(new_path).into_response(),
					)));
				}

				!self
					.config_flags
					.has(ConfigFlags::DROPS_ON_UNMATCHING_SLASH)
			} else {
				true
			};

			if handle {
				return match request_handler.as_ref() {
					MaybeBoxed::Boxed(boxed_request_handler) => {
						boxed_request_handler.handle(request_context, args)
					}
					MaybeBoxed::Unboxed(request_handler) => request_handler.handle(request_context, args),
				};
			}
		}

		handle_mistargeted_request(
			request_context,
			args,
			self.some_mistargeted_request_handler.as_ref(),
		)
	}
}

// ----------

#[derive(Clone)]
pub(crate) struct RequestPasser {
	some_static_resources: Option<Arc<[ResourceService]>>,
	some_regex_resources: Option<Arc<[ResourceService]>>,
	some_wildcard_resource: Option<Arc<ResourceService>>,

	some_mistargeted_request_handler: Option<ArcHandler>,
}

impl RequestPasser {
	pub(crate) fn new(
		some_static_resources: Option<Arc<[ResourceService]>>,
		some_regex_resources: Option<Arc<[ResourceService]>>,
		some_wildcard_resource: Option<Arc<ResourceService>>,
		some_mistargeted_request_handler: Option<ArcHandler>,
		middleware: &mut [LayerTarget<Resource>],
	) -> MaybeBoxed<Self> {
		let request_passer = Self {
			some_static_resources,
			some_regex_resources,
			some_wildcard_resource,
			some_mistargeted_request_handler,
		};

		let mut maybe_boxed_request_passer = MaybeBoxed::Unboxed(request_passer);

		for layer in middleware.iter_mut().rev() {
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
						let boxed_request_passer = BoxedHandler::new(request_passer);

						maybe_boxed_request_passer =
							MaybeBoxed::Boxed(boxed_layer.wrap(boxed_request_passer.into()));
					}
				}
			}
		}

		maybe_boxed_request_passer
	}
}

impl Handler for RequestPasser {
	type Response = Response;
	type Error = BoxedErrorResponse;
	type Future = BoxedFuture<Result<Self::Response, Self::Error>>;

	#[inline]
	fn handle(&self, mut request_context: RequestContext, mut args: Args) -> Self::Future {
		let some_next_resource = 'some_next_resource: {
			let (next_segment, uri_params) = request_context
				.routing_next_segment_and_uri_params_mut()
				.expect("request passer shouldn't be called when there is no next path segment");

			if let Some(next_resource) = self.some_static_resources.as_ref().and_then(|resources| {
				resources.iter().find(
					// Static patterns keep percent-encoded string. We may match them without
					// decoding the segment.
					|resource| {
						resource
							.pattern
							.is_static_match(next_segment)
							.expect("static_resources must keep only the resources with a static pattern")
					},
				)
			}) {
				break 'some_next_resource Some(next_resource);
			}

			let Ok(decoded_segment) = percent_decode_str(next_segment).decode_utf8() else {
				return Box::pin(ready(Ok(StatusCode::BAD_REQUEST.into_response())));
			};

			#[cfg(feature = "regex")]
			if let Some(next_resource) = self.some_regex_resources.as_ref().and_then(|resources| {
				resources.iter().find(|resource| {
					resource
						.pattern
						.is_regex_match(decoded_segment.as_ref(), uri_params)
						.expect("regex_resources must keep only the resources with a regex pattern")
				})
			}) {
				break 'some_next_resource Some(next_resource);
			}

			let _ = self
				.some_wildcard_resource
				.as_ref()
				.is_some_and(|resource| {
					resource
						.pattern
						.is_wildcard_match(decoded_segment, uri_params)
						.expect("wildcard_resource must keep only a resource with a wilcard pattern")
				});

			self.some_wildcard_resource.as_deref()
		};

		if let Some(next_resource) = some_next_resource {
			request_context.clone_valid_properties_from(&next_resource.context_properties);

			let args =
				args.extensions_replaced(NodeExtensions::new_borrowed(&next_resource.extensions), &());

			match &next_resource.request_receiver {
				MaybeBoxed::Boxed(boxed_request_receiver) => {
					return boxed_request_receiver.handle(request_context, args);
				}
				MaybeBoxed::Unboxed(request_receiver) => {
					return request_receiver.handle(request_context, args);
				}
			}
		}

		handle_mistargeted_request(
			request_context,
			args,
			self.some_mistargeted_request_handler.as_ref(),
		)
	}
}

// ----------

#[derive(Clone)]
pub(crate) struct RequestHandler {
	method_handlers: Vec<(Method, BoxedHandler)>,
	wildcard_method_handler: WildcardMethodHandler,
}

impl RequestHandler {
	pub(crate) fn new(
		method_handlers: Vec<(Method, BoxedHandler)>,
		wildcard_method_handler: WildcardMethodHandler,
		middleware: &mut [LayerTarget<Resource>],
		some_mistargeted_request_handler: Option<ArcHandler>,
	) -> Result<MaybeBoxed<Self>, Method> {
		let mut request_handler = Self {
			method_handlers,
			wildcard_method_handler: if wildcard_method_handler.is_none() {
				WildcardMethodHandler::None(some_mistargeted_request_handler)
			} else {
				wildcard_method_handler
			},
		};

		let mut request_handler_middleware_exists = false;

		for layer in middleware.iter_mut().rev() {
			match layer {
				LayerTarget::MethodHandler(..) => {
					let LayerTarget::MethodHandler(methods, boxed_layer) = layer.take() else {
						unreachable!()
					};

					for method in methods.into_iter().rev() {
						request_handler.wrap_method_handler(method, boxed_layer.clone())?
					}
				}
				LayerTarget::WildcardMethodHandler(_) => {
					let LayerTarget::WildcardMethodHandler(boxed_layer) = layer.take() else {
						unreachable!()
					};

					request_handler.wildcard_method_handler.wrap(boxed_layer);
				}
				LayerTarget::RequestHandler(_) => request_handler_middleware_exists = true,
				_ => {}
			}
		}

		if request_handler_middleware_exists {
			let mut boxed_request_handler = BoxedHandler::new(request_handler);

			for layer in middleware.iter_mut().rev() {
				if let LayerTarget::RequestHandler(_) = layer {
					let LayerTarget::RequestHandler(boxed_layer) = layer.take() else {
						unreachable!()
					};

					boxed_request_handler = boxed_layer.wrap(boxed_request_handler.into());
				}
			}

			Ok(MaybeBoxed::Boxed(boxed_request_handler))
		} else {
			Ok(MaybeBoxed::Unboxed(request_handler))
		}
	}

	pub(crate) fn wrap_method_handler(
		&mut self,
		method: Method,
		boxed_layer: BoxedLayer,
	) -> Result<(), Method> {
		let Some(position) = self.method_handlers.iter().position(|(m, _)| m == method) else {
			return Err(method);
		};

		let (method, boxed_handler) = std::mem::take(&mut self.method_handlers[position]);
		let boxed_handler = boxed_layer.wrap(boxed_handler.into());

		self.method_handlers[position] = (method, boxed_handler);

		Ok(())
	}
}

impl Handler for RequestHandler {
	type Response = Response;
	type Error = BoxedErrorResponse;
	type Future = BoxedFuture<Result<Self::Response, Self::Error>>;

	fn handle(&self, request_context: RequestContext, args: Args) -> Self::Future {
		let method = request_context.method_ref();
		let some_method_handler = self.method_handlers.iter().find(|(m, _)| m == method);

		if let Some((_, ref handler)) = some_method_handler {
			return handler.handle(request_context, args);
		}

		if method == Method::HEAD {
			let some_method_handler = self.method_handlers.iter().find(|(m, _)| m == Method::GET);

			if let Some((_, ref handler)) = some_method_handler {
				let response_future = handler.handle(request_context, args);

				return Box::pin(async {
					response_future.await.map(|mut response| {
						let _ = std::mem::take(response.body_mut());

						response
					})
				});
			}
		}

		self.wildcard_method_handler.handle(request_context, args)
	}
}

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

#[cfg(all(test, feature = "full"))]
mod test {
	use http::header::{ACCEPT_ENCODING, CONTENT_ENCODING};
	use http_body_util::{BodyExt, Empty};
	use tower_http::compression::CompressionLayer;

	use crate::{
		common::{
			config::_with_request_extensions_modifier,
			test_helpers::{new_root, test_service, Case, DataKind, Rx_1_1, Rx_2_0, Wl_3_0},
		},
		handler::{IntoHandler, _get, _post},
		middleware::{_request_handler, _request_passer, _request_receiver},
		request::RequestHead,
		resource::Resource,
	};

	use super::*;

	// --------------------------------------------------------------------------------
	// --------------------------------------------------------------------------------

	#[tokio::test]
	async fn resource_service() {
		//	/	->	/st_0_0	->	/{wl_1_0}	->	/{rx_2_0:p_0}/
		//							|							|	->	/{rx_2_1:p_1}-abc	->	/{wl_3_0}
		//							|
		//							|	->	/{rx_1_1:p_0}-abc/	->	/st_2_0
		//																			|	->	/st_2_1

		let cases = [
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

		let service = new_root().into_service();
		test_service(service, &cases).await;

		// -------------------------
		// non-root resource

		let mut resource = Resource::new("/st_0_0");
		resource.set_handler_for(_get.to(|| async {}));
		resource
			.subresource_mut("/{wl_1_0}")
			.set_handler_for(_get.to(|| async {}));

		let service = resource.into_service();

		// ----------

		let request = Request::builder()
			.method("GET")
			.uri("/st_0_0")
			.body(Empty::default())
			.unwrap();

		let response = service.call(request).await.unwrap();
		assert_eq!(response.status(), StatusCode::OK);

		// ----------

		let request = Request::builder()
			.method("GET")
			.uri("/st_0_0/wl_1_0")
			.body(Empty::default())
			.unwrap();

		let response = service.call(request).await.unwrap();
		assert_eq!(response.status(), StatusCode::OK);
	}

	// --------------------------------------------------
	// Middleware tests.

	#[derive(Clone)]
	struct Middleware;

	impl<H> Layer<H> for Middleware
	where
		H: Handler + Clone + Send + Sync,
		H::Response: IntoResponse,
		H::Error: Into<BoxedErrorResponse>,
	{
		type Handler = MiddlewareHandler<H>;

		fn wrap(&self, handler: H) -> Self::Handler {
			MiddlewareHandler(handler)
		}
	}

	#[derive(Clone)]
	struct MiddlewareHandler<H>(H);

	impl<B, H> Handler<B> for MiddlewareHandler<H>
	where
		H: Handler + Clone + Send + Sync,
		H::Response: IntoResponse,
		H::Error: Into<BoxedErrorResponse>,
	{
		type Response = Response;
		type Error = BoxedErrorResponse;
		type Future = BoxedFuture<Result<Self::Response, Self::Error>>;

		fn handle(&self, _request_context: RequestContext<B>, _args: Args<'_, ()>) -> Self::Future {
			Box::pin(ready(Ok("Hello from Middleware!".into_response())))
		}
	}

	#[tokio::test]
	async fn resource_handler_layer() {
		let mut root = Resource::new("/");
		root.subresource_mut("/st_0_0/st_1_0").set_handler_for([
			_get.to((|| async { "Hello from Handler!" }).wrapped_in(CompressionLayer::new())),
			_post.to(
				(|_: RequestHead, _: Args<'_, usize>| async { "Hello from Handler!" })
					.with_extension(42)
					.wrapped_in(Middleware),
			),
		]);

		// ----------

		let service = root.into_service();

		let request = Request::post("/st_0_0/st_1_0")
			.body(Empty::default())
			.unwrap();

		let response = service.call(request).await.unwrap();
		assert_eq!(response.status(), StatusCode::OK);

		let body = response.collect().await.unwrap().to_bytes();
		assert_eq!(body, "Hello from Middleware!");

		// ----------

		let request = Request::get("/st_0_0/st_1_0")
			.header(ACCEPT_ENCODING, "gzip")
			.body(Empty::default())
			.unwrap();

		let response = service.call(request).await.unwrap();
		assert_eq!(response.status(), StatusCode::OK);
		assert_eq!(response.headers()[CONTENT_ENCODING.as_str()], "gzip");
	}

	#[tokio::test]
	async fn resource_request_handler_layer() {
		let mut root = Resource::new("/");
		let st_1_0 = root.subresource_mut("/st_0_0/st_1_0");
		st_1_0.set_handler_for(_get.to(|| async { "Hello from Handler!" }));
		st_1_0.add_layer_to(_request_handler(Middleware));

		// ----------

		let service = root.into_service();

		let request = Request::builder()
			.uri("/st_0_0/st_1_0")
			.body(Empty::default())
			.unwrap();

		let response = service.call(request).await.unwrap();
		assert_eq!(response.status(), StatusCode::OK);

		let body = response.collect().await.unwrap().to_bytes();
		assert_eq!(body, "Hello from Middleware!");
	}

	#[tokio::test]
	async fn resource_request_handler_tower_layer() {
		let mut root = Resource::new("/");

		let st_1_0 = root.subresource_mut("/st_0_0/st_1_0");
		st_1_0.set_handler_for(_get.to(|| async { "Hello from Handler!" }));
		st_1_0.add_layer_to(_request_handler((CompressionLayer::new(), Middleware)));

		let st_1_1 = root.subresource_mut("/st_0_0/st_1_1");
		st_1_1.set_handler_for(_get.to(|| async { "Hello from Handler!" }));
		st_1_1.add_layer_to(_request_handler((Middleware, CompressionLayer::new())));

		// ----------

		let service = root.into_service();

		let request = Request::builder()
			.uri("/st_0_0/st_1_1")
			.header(ACCEPT_ENCODING, "gzip")
			.body(Empty::default())
			.unwrap();

		let response = service.call(request).await.unwrap();
		assert_eq!(response.status(), StatusCode::OK);

		let body = response.collect().await.unwrap().to_bytes();
		assert_eq!(body, "Hello from Middleware!");
	}

	#[tokio::test]
	async fn resource_request_passer_layer() {
		let mut root = Resource::new("/");

		root
			.subresource_mut("/st_0_0/st_1_0")
			.set_handler_for(_get.to(|| async { "Hello from Handler!" }));

		root
			.subresource_mut("/st_0_0/")
			.add_layer_to(_request_passer(Middleware));

		// ----------

		let service = root.into_service();

		let request = Request::builder()
			.uri("/st_0_0/st_1_0")
			.body(Empty::default())
			.unwrap();

		let response = service.call(request).await.unwrap();
		assert_eq!(response.status(), StatusCode::OK);

		let body = response.collect().await.unwrap().to_bytes();
		assert_eq!(body, "Hello from Middleware!");
	}

	#[tokio::test]
	async fn resource_request_receiver_layer() {
		let mut root = Resource::new("/");

		root
			.subresource_mut("/st_0_0/st_1_0")
			.set_handler_for(_get.to(|| async { "Hello from Handler!" }));

		root
			.subresource_mut("/st_0_0/")
			.add_layer_to(_request_receiver(Middleware));

		// ----------

		let service = root.into_service();

		let request = Request::builder()
			.uri("/st_0_0/st_1_0")
			.body(Empty::default())
			.unwrap();

		let response = service.call(request).await.unwrap();
		assert_eq!(response.status(), StatusCode::OK);

		let body = response.collect().await.unwrap().to_bytes();
		assert_eq!(body, "Hello from Middleware!");
	}

	// --------------------------------------------------
	// Request extensions test.

	#[tokio::test]
	async fn resource_request_extensions() {
		let mut root = Resource::new("/");
		root.configure(_with_request_extensions_modifier(|extensions| {
			extensions.insert("Hello from Handler!".to_string());
		}));

		root
			.subresource_mut("/st_0_0/st_1_0")
			.set_handler_for(_get.to(|head: RequestHead| async move {
				head.extensions_ref().get::<String>().unwrap().clone()
			}));

		// ----------

		let service = root.into_service();

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
