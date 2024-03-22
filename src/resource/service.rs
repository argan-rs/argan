use std::{
	any::Any,
	borrow::Cow,
	convert::Infallible,
	fmt::Debug,
	future::{ready, IntoFuture},
	pin::Pin,
	process::Output,
	sync::Arc,
};

use bytes::Bytes;
use http::{Extensions, Method, StatusCode, Uri};
use http_body_util::{BodyExt, Empty};
use percent_encoding::percent_decode_str;

use crate::{
	body::{Body, HttpBody},
	common::{mark::Private, BoxedError, BoxedFuture, MaybeBoxed, Uncloneable, SCOPE_VALIDITY},
	data::extensions::NodeExtensions,
	handler::{
		futures::ResponseToResultFuture,
		request_handlers::{
			self, handle_mistargeted_request, handle_unimplemented_method, MethodHandlers,
			MistargetedRequestHandler, UnimplementedMethodHandler, WildcardMethodHandler,
		},
		AdaptiveHandler, ArcHandler, Args, BoxedHandler, Handler, IntoHandler, Service,
	},
	middleware::{layer_targets::LayerTarget, BoxedLayer, Layer, ResponseResultFutureBoxer},
	pattern::{ParamsList, Pattern},
	request::Request,
	response::{BoxedErrorResponse, InfallibleResponseFuture, IntoResponse, Redirect, Response},
	routing::{self, RouteTraversal, RoutingState, UnusedRequest},
};

use super::{config::ConfigFlags, Resource};

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

#[derive(Clone)]
pub struct ResourceService {
	pattern: Pattern,
	extensions: Extensions,

	request_receiver: MaybeBoxed<RequestReceiver>,
	some_mistargeted_request_handler: Option<ArcHandler>,
}

impl ResourceService {
	#[inline(always)]
	pub(super) fn new(
		pattern: Pattern,
		extensions: Extensions,
		request_receiver: MaybeBoxed<RequestReceiver>,
		some_mistargeted_request_handler: Option<ArcHandler>,
	) -> Self {
		Self {
			pattern,
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
		request: Request<B>,
		args: &mut Args,
	) -> BoxedFuture<Result<Response, BoxedErrorResponse>>
	where
		B: HttpBody<Data = Bytes> + Send + Sync + 'static,
		B::Error: Into<BoxedError>,
	{
		let mut request = request.map(Body::new);
		let mut args = args.node_extensions_replaced(&self.extensions);

		match &self.request_receiver {
			MaybeBoxed::Boxed(boxed_request_receiver) => {
				boxed_request_receiver.handle(request, &mut args)
			}
			MaybeBoxed::Unboxed(request_receiver) => request_receiver.handle(request, &mut args),
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
		let mut request = request.map(Body::new);

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

				if let Some(result) = self
					.pattern
					.is_regex_match(decoded_segment.as_ref(), &mut path_params)
				{
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

		let mut args = Args {
			routing_state,
			node_extensions: NodeExtensions::new_borrowed(&self.extensions),
			handler_extension: &(),
		};

		if matched {
			match &self.request_receiver {
				MaybeBoxed::Boxed(boxed_request_receiver) => {
					InfallibleResponseFuture::from(boxed_request_receiver.handle(request, &mut args))
				}
				MaybeBoxed::Unboxed(request_receiver) => {
					InfallibleResponseFuture::from(request_receiver.handle(request, &mut args))
				}
			}
		} else {
			InfallibleResponseFuture::from(handle_mistargeted_request(
				request,
				args.routing_state,
				self
					.some_mistargeted_request_handler
					.as_ref()
					.map(|handler| (handler, args.node_extensions)),
			))
		}
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

		for layer in middleware {
			if let LayerTarget::RequestReceiver(boxed_layer) = layer {
				match maybe_boxed_request_receiver {
					MaybeBoxed::Boxed(mut boxed_request_receiver) => {
						maybe_boxed_request_receiver =
							MaybeBoxed::Boxed(boxed_layer.wrap(boxed_request_receiver.into()));
					}
					MaybeBoxed::Unboxed(request_receiver) => {
						let mut boxed_request_receiver = BoxedHandler::new(request_receiver);

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
	fn handle(&self, mut request: Request, args: &mut Args) -> Self::Future {
		if args
			.routing_state
			.route_traversal
			.has_remaining_segments(request.uri().path())
		{
			let resource_is_subtree_handler = self.is_subtree_hander();

			if let Some(request_passer) = self.some_request_passer.as_ref() {
				if resource_is_subtree_handler {
					args.routing_state.subtree_handler_exists = true;
				}

				let next_segment_index = args.routing_state.route_traversal.next_segment_index();

				let response_future = match request_passer {
					MaybeBoxed::Boxed(boxed_request_passer) => boxed_request_passer.handle(request, args),
					MaybeBoxed::Unboxed(request_passer) => request_passer.handle(request, args),
				};

				if !resource_is_subtree_handler {
					return response_future;
				}

				let request_handler_clone = self
					.some_request_handler
					.clone()
					.expect("subtree handler must have a request handler");

				let node_extensions = args.node_extensions.clone().into_owned();

				return Box::pin(async move {
					let mut response = response_future.await?;
					if response.status() != StatusCode::NOT_FOUND {
						return Ok(response);
					}

					let Some(uncloneable) = response
						.extensions_mut()
						.remove::<Uncloneable<UnusedRequest>>()
					else {
						// Custom 404 Not Found response.
						return Ok(response);
					};

					let mut request = uncloneable
						.into_inner()
						.expect("unused request should always exist in Uncloneable")
						.into_request();

					let mut routing_state = response
						.extensions_mut()
						.remove::<Uncloneable<RoutingState>>()
						.expect("Uncloneable<RoutingState> should always exist when unused request is returned")
						.into_inner()
						.expect("RoutingState should always exist in Uncloneable");

					routing_state
						.route_traversal
						.revert_to_segment(next_segment_index);

					let mut args = Args {
						routing_state,
						node_extensions,
						handler_extension: &(),
					};

					match request_handler_clone.as_ref() {
						MaybeBoxed::Boxed(boxed_request_handler) => {
							boxed_request_handler.handle(request, &mut args).await
						}
						MaybeBoxed::Unboxed(request_handler) => {
							request_handler.handle(request, &mut args).await
						}
					}
				});
			}

			if !resource_is_subtree_handler {
				let routing_state = std::mem::take(&mut args.routing_state);
				let node_extensions = args.node_extensions.take();

				return handle_mistargeted_request(
					request,
					routing_state,
					self
						.some_mistargeted_request_handler
						.as_ref()
						.map(|handler| (handler, node_extensions)),
				);
			}
		}

		if let Some(request_handler) = self.some_request_handler.as_ref() {
			let request_path_ends_with_slash = args
				.routing_state
				.route_traversal
				.ends_with_slash(request.uri().path());

			let resource_path_ends_with_slash = self.config_flags.has(ConfigFlags::ENDS_WITH_SLASH);

			let handle = if request_path_ends_with_slash && !resource_path_ends_with_slash {
				if self
					.config_flags
					.has(ConfigFlags::REDIRECTS_ON_UNMATCHING_SLASH)
				{
					let path = request.uri().path();

					return Box::pin(ready(Ok(
						Redirect::permanently(&path[..path.len() - 1]).into_response(),
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
					let path = request.uri().path();

					let mut new_path = String::with_capacity(path.len() + 1);
					new_path.push_str(path);
					new_path.push('/');

					return Box::pin(ready(Ok(Redirect::permanently(new_path).into_response())));
				}

				!self
					.config_flags
					.has(ConfigFlags::DROPS_ON_UNMATCHING_SLASH)
			} else {
				true
			};

			if handle {
				return match request_handler.as_ref() {
					MaybeBoxed::Boxed(boxed_request_handler) => boxed_request_handler.handle(request, args),
					MaybeBoxed::Unboxed(request_handler) => request_handler.handle(request, args),
				};
			}
		}

		let routing_state = std::mem::take(&mut args.routing_state);
		let node_extensions = args.node_extensions.take();

		handle_mistargeted_request(
			request,
			routing_state,
			self
				.some_mistargeted_request_handler
				.as_ref()
				.map(|handler| (handler, node_extensions)),
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
		middleware: &mut Vec<LayerTarget<Resource>>,
	) -> MaybeBoxed<Self> {
		let request_passer = Self {
			some_static_resources,
			some_regex_resources,
			some_wildcard_resource,
			some_mistargeted_request_handler,
		};

		let mut maybe_boxed_request_passer = MaybeBoxed::Unboxed(request_passer);

		for layer in middleware.iter_mut().rev() {
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
							let boxed_request_passer = BoxedHandler::new(request_passer);

							maybe_boxed_request_passer =
								MaybeBoxed::Boxed(boxed_layer.wrap(boxed_request_passer.into()));
						}
					}
				}
				_ => {}
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
	fn handle(&self, mut request: Request, args: &mut Args) -> Self::Future {
		let some_next_resource = 'some_next_resource: {
			let (next_segment, _) = args
				.routing_state
				.route_traversal
				.next_segment(request.uri().path())
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

			if let Some(next_resource) = self.some_regex_resources.as_ref().and_then(|resources| {
				resources.iter().find(|resource| {
					resource
						.pattern
						.is_regex_match(decoded_segment.as_ref(), &mut args.routing_state.uri_params)
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
						.is_wildcard_match(decoded_segment, &mut args.routing_state.uri_params)
						.expect("wildcard_resource must keep only a resource with a wilcard pattern")
				});

			self.some_wildcard_resource.as_deref()
		};

		if let Some(next_resource) = some_next_resource {
			let mut args = args.node_extensions_replaced(&next_resource.extensions);

			match &next_resource.request_receiver {
				MaybeBoxed::Boxed(boxed_request_receiver) => {
					return boxed_request_receiver.handle(request, &mut args);
				}
				MaybeBoxed::Unboxed(request_receiver) => {
					return request_receiver.handle(request, &mut args);
				}
			}
		}

		let routing_state = std::mem::take(&mut args.routing_state);
		let node_extensions = args.node_extensions.take();

		handle_mistargeted_request(
			request,
			routing_state,
			self
				.some_mistargeted_request_handler
				.as_ref()
				.map(|handler| (handler, node_extensions)),
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
		middleware: &mut Vec<LayerTarget<Resource>>,
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
						if let Err(method) = request_handler.wrap_method_handler(method, boxed_layer.clone()) {
							return Err(method);
						}
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

	fn handle(&self, request: Request, args: &mut Args) -> Self::Future {
		let method = request.method();
		let some_method_handler = self.method_handlers.iter().find(|(m, _)| m == method);

		if let Some((_, ref handler)) = some_method_handler {
			return handler.handle(request, args);
		}

		if method == Method::HEAD {
			let some_method_handler = self.method_handlers.iter().find(|(m, _)| m == Method::GET);

			if let Some((_, ref handler)) = some_method_handler {
				let response_future = handler.handle(request, args);

				return Box::pin(async {
					response_future.await.map(|mut response| {
						let _ = std::mem::replace(response.body_mut(), Body::default());

						response
					})
				});
			}
		}

		self.wildcard_method_handler.handle(request, args)
	}
}

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

#[cfg(test)]
mod test {
	use std::future::Ready;

	use http_body_util::Empty;

	use crate::{
		common::{
			config::_with_request_extensions_modifier,
			test_helpers::{new_root, test_service, Case, DataKind, Rx_1_1, Rx_2_0, Wl_3_0},
		},
		handler::{DummyHandler, IntoExtendedHandler, IntoWrappedHandler, _get},
		middleware::{IntoResponseResultAdapter, _request_handler, _request_passer, _request_receiver},
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
		resource.set_handler_for(_get(|| async {}));
		resource
			.subresource_mut("/{wl_1_0}")
			.set_handler_for(_get(|| async {}));

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

		fn handle(&self, request: Request<B>, args: &mut Args<'_, ()>) -> Self::Future {
			Box::pin(ready(Ok("Hello from Middleware!".into_response())))
		}
	}

	#[tokio::test]
	async fn resource_handler_layer() {
		let mut root = Resource::new("/");
		root.subresource_mut("/st_0_0/st_1_0").set_handler_for(_get(
			(|| async { "Hello from Handler!" })
				.with_extension(42)
				.wrapped_in(Middleware),
		));

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
	async fn resource_request_handler_layer() {
		let mut root = Resource::new("/");
		let mut st_1_0 = root.subresource_mut("/st_0_0/st_1_0");
		st_1_0.set_handler_for(_get(|| async { "Hello from Handler!" }));
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
	async fn resource_request_passer_layer() {
		let mut root = Resource::new("/");

		root
			.subresource_mut("/st_0_0/st_1_0")
			.set_handler_for(_get(|| async { "Hello from Handler!" }));

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
			.set_handler_for(_get(|| async { "Hello from Handler!" }));

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
			.set_handler_for(_get(|request: Request| async move {
				request.extensions().get::<String>().unwrap().clone()
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
