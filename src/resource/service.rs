use std::{
	any::Any, convert::Infallible, fmt::Debug, future::IntoFuture, process::Output, sync::Arc,
};

use http::{Method, StatusCode};
use percent_encoding::percent_decode_str;

use crate::{
	body::{Body, IncomingBody},
	common::{mark::Private, BoxedError, BoxedFuture, MaybeBoxed, Uncloneable},
	extension::Extensions,
	handler::{
		futures::ResponseToResultFuture,
		request_handlers::{
			self, handle_mistargeted_request, handle_unimplemented_method, MethodHandlers,
			MistargetedRequestHandler, UnimplementedMethodHandler,
		},
		AdaptiveHandler, BoxedHandler, FinalHandler, Handler, IntoHandler, Service,
	},
	middleware::{BoxedLayer, LayerTarget, ResponseFutureBoxer},
	pattern::{ParamsList, Pattern},
	request::Request,
	response::Response,
	routing::{RouteTraversal, RoutingState, UnusedRequest},
};

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

#[derive(Clone)]
pub struct ResourceService {
	pub(super) pattern: Pattern,
	pub(super) extensions: Extensions,
	pub(super) is_subtree_handler: bool,

	pub(super) request_receiver: MaybeBoxed<RequestReceiver>,
	pub(super) some_mistargeted_request_handler: Option<BoxedHandler>,
}

impl ResourceService {
	#[inline(always)]
	pub(crate) fn new(
		pattern: Pattern,
		extensions: Extensions,
		is_subtree_handler: bool,
		request_receiver: MaybeBoxed<RequestReceiver>,
		some_mistargeted_request_handler: Option<BoxedHandler>,
	) -> Self {
		Self {
			pattern,
			extensions,
			is_subtree_handler,
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
	pub(super) fn is_subtree_handler(&self) -> bool {
		self.is_subtree_handler
	}
}

// --------------------------------------------------

impl<B> Service<Request<B>> for ResourceService
where
	B: Body + Send + Sync + 'static,
	B::Data: Debug,
	B::Error: Into<BoxedError>,
{
	type Response = Response;
	type Error = Infallible;
	type Future = BoxedFuture<Result<Response, Infallible>>;

	#[inline]
	fn call(&self, request: Request<B>) -> Self::Future {
		let (head, body) = request.into_parts();
		let incoming_body = IncomingBody::new(body);
		let mut request = Request::<IncomingBody>::from_parts(head, incoming_body);

		let route = request.uri().path();
		let mut route_traversal = RouteTraversal::for_route(route);
		let mut path_params = ParamsList::new();

		let matched = if route == "/" {
			self.is_root()
		} else if self.is_root() {
			// Resource is a root and the request's path always starts from root.
			true
		} else {
			let (next_segment, _) = route_traversal
				.next_segment(route)
				.expect("route should contain a path segment");

			// If pattern is static, we may match it without decoding the segment.
			// Static patterns keep percent-encoded string.
			if let Some(result) = self.pattern.is_static_match(next_segment) {
				result
			} else {
				let decoded_segment = Arc::<str>::from(
					percent_decode_str(next_segment)
						.decode_utf8()
						.expect("decoded segment should be a valid utf8 string"), // ???
				);

				if let Some(result) = self
					.pattern
					.is_regex_match(decoded_segment.clone(), &mut path_params)
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
		routing_state.path_params = path_params;

		if matched {
			Box::pin(ResponseToResultFuture::from(match &self.request_receiver {
				MaybeBoxed::Boxed(boxed_request_receiver) => {
					request
						.extensions_mut()
						.insert(Uncloneable::from(routing_state));

					boxed_request_receiver.handle(request)
				}
				MaybeBoxed::Unboxed(request_receiver) => {
					request_receiver.handle_with_routing_state(request, routing_state)
				}
			}))
		} else {
			Box::pin(ResponseToResultFuture::from(handle_mistargeted_request(
				request,
				routing_state,
				self.some_mistargeted_request_handler.as_ref(),
			)))
		}
	}
}

// --------------------------------------------------

#[derive(Clone)]
pub(crate) struct RequestReceiver {
	pub(super) some_request_passer: Option<MaybeBoxed<RequestPasser>>,
	pub(super) some_request_handler: Option<Arc<MaybeBoxed<RequestHandler>>>,
	pub(super) some_mistargeted_request_handler: Option<BoxedHandler>,

	pub(super) is_subtree_handler: bool,
}

impl RequestReceiver {
	pub(crate) fn new(
		some_request_passer: Option<MaybeBoxed<RequestPasser>>,
		some_request_handler: Option<Arc<MaybeBoxed<RequestHandler>>>,
		some_mistargeted_request_handler: Option<BoxedHandler>,
		is_subtree_handler: bool,
		middleware: Vec<LayerTarget>,
	) -> MaybeBoxed<Self> {
		let request_receiver = Self {
			some_request_passer,
			some_request_handler,
			some_mistargeted_request_handler,
			is_subtree_handler,
		};

		let mut maybe_boxed_request_receiver = MaybeBoxed::from_unboxed(request_receiver);

		for layer in middleware {
			use crate::middleware::Inner;

			if let Inner::RequestReceiver(boxed_layer) = layer.0 {
				match maybe_boxed_request_receiver {
					MaybeBoxed::Boxed(mut boxed_request_receiver) => {
						maybe_boxed_request_receiver =
							MaybeBoxed::Boxed(boxed_layer.wrap(boxed_request_receiver.into()));
					}
					MaybeBoxed::Unboxed(request_receiver) => {
						let mut boxed_request_receiver =
							ResponseFutureBoxer::wrap(request_receiver).into_boxed_handler();

						maybe_boxed_request_receiver =
							MaybeBoxed::Boxed(boxed_layer.wrap(boxed_request_receiver.into()));
					}
				}
			}
		}

		maybe_boxed_request_receiver
	}

	pub(crate) fn handle_with_routing_state(
		&self,
		mut request: Request,
		mut routing_state: RoutingState,
	) -> BoxedFuture<Response> {
		if routing_state
			.path_traversal
			.has_remaining_segments(request.uri().path())
		{
			if let Some(request_passer) = self.some_request_passer.as_ref() {
				if self.is_subtree_handler {
					routing_state.subtree_handler_exists = true;
				}

				let next_segment_index = routing_state.path_traversal.next_segment_index();

				let response_future = match request_passer {
					MaybeBoxed::Boxed(boxed_request_passer) => {
						request
							.extensions_mut()
							.insert(Uncloneable::from(routing_state));

						boxed_request_passer.handle(request).into()
					}
					MaybeBoxed::Unboxed(request_passer) => {
						request_passer.handle_with_routing_state(request, routing_state)
					}
				};

				if !self.is_subtree_handler {
					return response_future;
				}

				let request_handler_clone = self
					.some_request_handler
					.clone()
					.expect("subtree handler must have a request handler");

				return /* ResponseFuture::from( */Box::pin(async move {
					let mut response = response_future.await;
					if response.status() != StatusCode::NOT_FOUND {
						return response;
					}

					let Some(uncloneable) = response
						.extensions_mut()
						.remove::<Uncloneable<UnusedRequest>>()
					else {
						// Custom 404 Not Found response.
						return response;
					};

					let mut request = uncloneable
						.into_inner()
						.expect("unused request should always exist in Uncloneable")
						.into_request();

					let mut routing_state = request
						.extensions_mut()
						.get_mut::<Uncloneable<RoutingState>>()
						.expect("Uncloneable<RoutingState> should always exist in a request")
						.as_mut()
						.expect("RoutingState should always exist in Uncloneable");

					routing_state
						.path_traversal
						.revert_to_segment(next_segment_index);

					match request_handler_clone.as_ref() {
						MaybeBoxed::Boxed(boxed_request_handler) => boxed_request_handler.handle(request).await,
						MaybeBoxed::Unboxed(request_handler) => request_handler.handle(request).await,
					}
				})/*  as BoxedFuture<Response>) */;
			}

			if !self.is_subtree_handler {
				return handle_mistargeted_request(
					request,
					routing_state,
					self.some_mistargeted_request_handler.as_ref(),
				);
			}
		}

		if let Some(request_handler) = self.some_request_handler.as_ref() {
			request
				.extensions_mut()
				.insert(Uncloneable::from(routing_state));

			return match request_handler.as_ref() {
				MaybeBoxed::Boxed(boxed_request_handler) => boxed_request_handler.handle(request),
				MaybeBoxed::Unboxed(request_handler) => request_handler.handle(request),
			};
		}

		handle_mistargeted_request(
			request,
			routing_state,
			self.some_mistargeted_request_handler.as_ref(),
		)
	}
}

impl Handler for RequestReceiver {
	type Response = Response;
	type Future = BoxedFuture<Response>;

	fn handle(&self, mut request: Request<IncomingBody>) -> Self::Future {
		let mut routing_state = request
			.extensions_mut()
			.remove::<Uncloneable<RoutingState>>()
			.expect("Uncloneable<RoutingState> should always exist in a request")
			.into_inner()
			.expect("RoutingState should always exist in Uncloneable");

		self.handle_with_routing_state(request, routing_state)
	}
}

// ----------

#[derive(Clone)]
pub(crate) struct RequestPasser {
	pub(super) some_static_resources: Option<Arc<[ResourceService]>>,
	pub(super) some_regex_resources: Option<Arc<[ResourceService]>>,
	pub(super) some_wildcard_resource: Option<Arc<ResourceService>>,

	pub(super) some_mistargeted_request_handler: Option<BoxedHandler>,
}

impl RequestPasser {
	pub(crate) fn new(
		some_static_resources: Option<Arc<[ResourceService]>>,
		some_regex_resources: Option<Arc<[ResourceService]>>,
		some_wildcard_resource: Option<Arc<ResourceService>>,
		some_mistargeted_request_handler: Option<BoxedHandler>,
		middleware: &mut Vec<LayerTarget>,
	) -> MaybeBoxed<Self> {
		let request_passer = Self {
			some_static_resources,
			some_regex_resources,
			some_wildcard_resource,
			some_mistargeted_request_handler,
		};

		let mut maybe_boxed_request_passer = MaybeBoxed::from_unboxed(request_passer);

		for layer in middleware.iter_mut().rev() {
			use crate::middleware::Inner;

			match &mut layer.0 {
				Inner::RequestPasser(_) => {
					let Inner::RequestPasser(boxed_layer) = layer.0.take() else {
						unreachable!()
					};

					match maybe_boxed_request_passer {
						MaybeBoxed::Boxed(boxed_request_passer) => {
							maybe_boxed_request_passer =
								MaybeBoxed::from_boxed(boxed_layer.wrap(boxed_request_passer.into()))
						}
						MaybeBoxed::Unboxed(request_passer) => {
							let boxed_request_passer =
								ResponseFutureBoxer::wrap(request_passer).into_boxed_handler();

							maybe_boxed_request_passer =
								MaybeBoxed::from_boxed(boxed_layer.wrap(boxed_request_passer.into()));
						}
					}
				}
				_ => {}
			}
		}

		maybe_boxed_request_passer
	}

	pub(crate) fn handle_with_routing_state(
		&self,
		mut request: Request,
		mut routing_state: RoutingState,
	) -> BoxedFuture<Response> {
		let some_next_resource = 'some_next_resource: {
			let (next_segment, _) = routing_state
				.path_traversal
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

			let decoded_segment = Arc::<str>::from(
				percent_decode_str(next_segment)
					.decode_utf8()
					.expect("decoded segment should be a valid utf8 string"), // ???
			);

			if let Some(next_resource) = self.some_regex_resources.as_ref().and_then(|resources| {
				resources.iter().find(|resource| {
					resource
						.pattern
						.is_regex_match(decoded_segment.clone(), &mut routing_state.path_params)
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
						.is_wildcard_match(decoded_segment, &mut routing_state.path_params)
						.expect("wildcard_resource must keep only a resource with a wilcard pattern")
				});

			self.some_wildcard_resource.as_deref()
		};

		if let Some(next_resource) = some_next_resource {
			match &next_resource.request_receiver {
				MaybeBoxed::Boxed(boxed_request_receiver) => {
					request
						.extensions_mut()
						.insert(Uncloneable::from(routing_state));

					return boxed_request_receiver.handle(request);
				}
				MaybeBoxed::Unboxed(request_receiver) => {
					return request_receiver.handle_with_routing_state(request, routing_state)
				}
			}
		}

		handle_mistargeted_request(
			request,
			routing_state,
			self.some_mistargeted_request_handler.as_ref(),
		)
	}
}

impl Handler for RequestPasser {
	type Response = Response;
	type Future = BoxedFuture<Response>;

	fn handle(&self, mut request: Request<IncomingBody>) -> Self::Future {
		let mut routing_state = request
			.extensions_mut()
			.remove::<Uncloneable<RoutingState>>()
			.expect("Uncloneable<RoutingState> should be inserted before request_passer is called")
			.into_inner()
			.expect("RoutingState should always exist in Uncloneable");

		self.handle_with_routing_state(request, routing_state)
	}
}

// ----------

#[derive(Clone)]
pub(crate) struct RequestHandler {
	pub(super) allowed_methods: String,

	pub(super) method_handlers: Vec<(Method, BoxedHandler)>,
	pub(super) some_wildcard_method_handler: Option<BoxedHandler>,
}

impl RequestHandler {
	pub(crate) fn new(
		method_handlers: MethodHandlers,
		middleware: &mut Vec<LayerTarget>,
	) -> Result<MaybeBoxed<Self>, Method> {
		let MethodHandlers {
			method_handlers,
			some_wildcard_method_handler,
			send_allowed_methods,
			mut allowed_methods,
		} = method_handlers;

		if !send_allowed_methods {
			allowed_methods = String::new();
		}

		let mut request_handler = Self {
			allowed_methods,
			method_handlers,
			some_wildcard_method_handler,
		};

		let mut request_handler_middleware_exists = false;

		use crate::middleware::Inner;

		for layer in middleware.iter_mut().rev() {
			match &mut layer.0 {
				Inner::MethodHandler(..) => {
					let Inner::MethodHandler(methods, boxed_layer) = layer.0.take() else {
						unreachable!()
					};

					for method in methods.into_iter().rev() {
						if let Err(method) =
							request_handler.wrap_method_handler(method, boxed_layer.boxed_clone())
						{
							return Err(method);
						}
					}
				}
				Inner::WildcardMethodHandler(_) => {
					let Inner::WildcardMethodHandler(boxed_layer) = layer.0.take() else {
						unreachable!()
					};

					request_handler.wrap_wildcard_method_handler(boxed_layer)
				}
				Inner::RequestHandler(_) => request_handler_middleware_exists = true,
				_ => {}
			}
		}

		if request_handler_middleware_exists {
			let mut boxed_request_handler =
				ResponseFutureBoxer::wrap(request_handler).into_boxed_handler();

			for layer in middleware.iter_mut().rev() {
				if let Inner::RequestHandler(_) = &mut layer.0 {
					let Inner::RequestHandler(boxed_layer) = layer.0.take() else {
						unreachable!()
					};

					boxed_request_handler = boxed_layer.wrap(boxed_request_handler.into());
				}
			}

			Ok(MaybeBoxed::Boxed(boxed_request_handler))
		} else {
			Ok(MaybeBoxed::from_unboxed(request_handler))
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

	pub(crate) fn wrap_wildcard_method_handler(&mut self, boxed_layer: BoxedLayer) {
		let boxed_handler = match self.some_wildcard_method_handler.take() {
			Some(wildcard_method_handler) => wildcard_method_handler,
			None => {
				let allowed_methods = std::mem::take(&mut self.allowed_methods);
				let unimplemented_method_handler = UnimplementedMethodHandler::new(allowed_methods);

				ResponseFutureBoxer::wrap(unimplemented_method_handler).into_boxed_handler()
			}
		};

		let boxed_handler = boxed_layer.wrap(boxed_handler.into());

		self.some_wildcard_method_handler.replace(boxed_handler);
	}
}

impl Handler for RequestHandler {
	type Response = Response;
	type Future = BoxedFuture<Response>;

	fn handle(&self, request: Request<IncomingBody>) -> Self::Future {
		let method = request.method().clone();
		let some_method_handler = self.method_handlers.iter().find(|(m, _)| m == method);

		if let Some((_, ref handler)) = some_method_handler {
			handler.handle(request).into()
		} else {
			if let Some(wildcard_method_handler) = self.some_wildcard_method_handler.as_ref() {
				wildcard_method_handler.handle(request)
			} else {
				handle_unimplemented_method(request, &self.allowed_methods)
			}
		}
	}
}

// --------------------------------------------------------------------------------

// #[inline(always)]
// pub(super) fn request_receiver(request: Request) -> RequestReceiverFuture {
// 	RequestReceiverFuture::from(request)
// }
//
// #[inline(always)]
// pub(super) fn request_passer(request: Request) -> RequestPasserFuture {
// 	RequestPasserFuture::from(request)
// }
//
// #[inline(always)]
// pub(super) fn request_handler(request: Request) -> BoxedFuture<Response> {
// 	let routing_state = request
// 		.extensions()
// 		.get::<Uncloneable<RoutingState>>()
// 		.expect("Uncloneable<RoutingState> should be inserted before request_handler is called")
// 		.as_ref()
// 		.expect("RoutingState should always exist in Uncloneable");
//
// 	let current_resource = routing_state.current_resource.clone().expect(
// 		"current resource should be set in the request_passer or the call method of the Service",
// 	);
//
// 	current_resource.0.method_handlers.handle(request)
// }

// #[cfg(test)]
// mod test {
// 	use std::str::FromStr;
//
// 	use http::{header::CONTENT_TYPE, Method, StatusCode, Uri};
// 	use http_body_util::BodyExt;
// 	use serde::{Deserialize, Serialize};
//
// 	use crate::{
// 		body::{Bytes, Empty},
// 		data::Json,
// 		handler::{get, method, post},
// 		request::PathParam,
// 		resource::Resource,
// 	};
//
// 	use super::*;
//
// 	// --------------------------------------------------
//
// 	#[tokio::test]
// 	async fn resource_service() {
// 		let mut root = Resource::new("/");
// 		let handler = |_request: Request| async {};
// 		root.subresource_mut("/abc").set_handler(get(handler));
// 		assert_eq!(root.subresource_mut("/abc").pattern(), "abc");
// 		assert!(root.subresource_mut("/abc").can_handle_request());
//
// 		let service = root.into_service();
// 		let static_resource = &service.some_static_resources.as_ref().unwrap();
// 		assert_eq!(static_resource.len(), 1);
// 		assert_eq!(static_resource[0].0.pattern.to_string(), "abc");
//
// 		let request = Request::get("/abc").body(Empty::<Bytes>::new()).unwrap();
// 		let response = service.call(request).await.unwrap();
// 		assert_eq!(response.status(), StatusCode::OK);
//
// 		// --------------------------------------------------
// 		// --------------------------------------------------
// 		//		abc0_0 -> *abc1_0 -> $abc2_0:@(abc2_0)
// 		//					 |					-> $abc2_1:@cn(abc2_1)-cba -> *abc3_0
// 		//					 |
// 		//					 -> $abc1_1:@cn(abc1_1)-cba -> abc2_0
//
// 		let mut resource = Resource::new("/abc0_0");
// 		resource.set_handler(get(hello_world));
//
// 		resource.subresource_mut("/*abc1_0").set_handler(method(
// 			"PUT",
// 			|PathParam(wildcard): PathParam<String>| async move {
// 				println!("got param: {}", wildcard);
//
// 				wildcard
// 			},
// 		));
//
// 		resource
// 			.subresource_mut("/*abc1_0/$abc2_0:@(abc2_0)")
// 			.set_handler(post(
// 				|PathParam(path_values): PathParam<PathValues1_0_2_0>| async move {
// 					println!("got path values: {:?}", path_values);
//
// 					Json(path_values)
// 				},
// 			));
//
// 		#[derive(Debug, Serialize, Deserialize)]
// 		struct PathValues1_0_2_0 {
// 			abc1_0: String,
// 			abc2_0: Option<String>,
// 			abc3_0: Option<u64>,
// 		}
//
// 		resource
// 			.subresource_mut("/*abc1_0/$abc2_1:@cn(abc2_1)-cba/*abc3_0")
// 			.set_handler(get(
// 				|PathParam(path_values): PathParam<PathValues1_0_2_1_3_0>| async move {
// 					println!("got path values: {:?}", path_values);
//
// 					Json(path_values)
// 				},
// 			));
//
// 		#[derive(Debug, Serialize, Deserialize)]
// 		struct PathValues1_0_2_1_3_0 {
// 			abc1_0: Option<String>,
// 			abc2_1: String,
// 			abc3_0: u64,
// 		}
//
// 		resource
// 			.subresource_mut("/$abc1_1:@cn(abc1_1)-cba")
// 			.set_handler(get(|PathParam(value): PathParam<String>| async move {
// 				let vector = Vec::from(value);
// 				println!("got path values: {:?}", vector);
//
// 				vector
// 			}));
//
// 		resource
// 			.subresource_mut("/$abc1_1:@cn(abc1_1)-cba/abc2_0")
// 			.set_handler(get(hello_world));
//
// 		dbg!();
//
// 		let service = resource.into_service();
//
// 		dbg!();
//
// 		let request = new_request("GET", "/abc0_0");
// 		let response = service.call(request).await.unwrap();
// 		assert_eq!(response.status(), StatusCode::OK);
//
// 		dbg!();
//
// 		let request = new_request("PUT", "/abc0_0/abc1_0");
// 		let response = service.call(request).await.unwrap();
// 		assert_eq!(response.status(), StatusCode::OK);
// 		assert_eq!(
// 			response
// 				.headers()
// 				.get(CONTENT_TYPE)
// 				.unwrap()
// 				.to_str()
// 				.unwrap(),
// 			mime::TEXT_PLAIN_UTF_8,
// 		);
//
// 		let body = response.into_body().collect().await.unwrap().to_bytes();
// 		assert_eq!(body.as_ref(), "abc1_0".as_bytes());
//
// 		dbg!();
//
// 		let request = new_request("POST", "/abc0_0/abc1_0/abc2_0");
// 		let response = service.call(request).await.unwrap();
// 		assert_eq!(response.status(), StatusCode::OK);
// 		assert_eq!(
// 			response
// 				.headers()
// 				.get(CONTENT_TYPE)
// 				.unwrap()
// 				.to_str()
// 				.unwrap(),
// 			mime::APPLICATION_JSON,
// 		);
//
// 		let json_body = String::from_utf8(
// 			response
// 				.into_body()
// 				.collect()
// 				.await
// 				.unwrap()
// 				.to_bytes()
// 				.to_vec(),
// 		)
// 		.unwrap();
// 		assert_eq!(
// 			json_body,
// 			r#"{"abc1_0":"abc1_0","abc2_0":"abc2_0","abc3_0":null}"#
// 		);
//
// 		dbg!();
//
// 		let request = new_request("GET", "/abc0_0/abc1_1-cba");
// 		let response = service.call(request).await.unwrap();
// 		assert_eq!(response.status(), StatusCode::OK);
// 		assert_eq!(
// 			response
// 				.headers()
// 				.get(CONTENT_TYPE)
// 				.unwrap()
// 				.to_str()
// 				.unwrap(),
// 			mime::APPLICATION_OCTET_STREAM,
// 		);
//
// 		let vector_body = response
// 			.into_body()
// 			.collect()
// 			.await
// 			.unwrap()
// 			.to_bytes()
// 			.to_vec();
// 		assert_eq!(vector_body, b"abc1_1".to_vec());
//
// 		dbg!();
//
// 		let request = new_request("GET", "/abc0_0/abc1_1-cba/abc2_0");
// 		let response = service.call(request).await.unwrap();
// 		assert_eq!(response.status(), StatusCode::OK);
//
// 		dbg!();
//
// 		let request = new_request("GET", "/abc0_0/abc1_0-wildcard/abc2_1-cba/30");
// 		let response = service.call(request).await.unwrap();
// 		assert_eq!(response.status(), StatusCode::OK);
// 		assert_eq!(
// 			response
// 				.headers()
// 				.get(CONTENT_TYPE)
// 				.unwrap()
// 				.to_str()
// 				.unwrap(),
// 			mime::APPLICATION_JSON,
// 		);
//
// 		let json_body = String::from_utf8(
// 			response
// 				.into_body()
// 				.collect()
// 				.await
// 				.unwrap()
// 				.to_bytes()
// 				.to_vec(),
// 		)
// 		.unwrap();
// 		assert_eq!(
// 			json_body,
// 			r#"{"abc1_0":"abc1_0-wildcard","abc2_1":"abc2_1","abc3_0":30}"#
// 		);
// 	}
//
// 	fn new_request(method: &str, uri: &str) -> Request<Empty<Bytes>> {
// 		let mut request = Request::new(Empty::<Bytes>::new());
// 		*request.method_mut() = Method::from_str(method).unwrap();
// 		*request.uri_mut() = Uri::from_str(uri).unwrap();
//
// 		request
// 	}
//
// 	async fn hello_world() -> &'static str {
// 		"Hello, World!"
// 	}
// }
