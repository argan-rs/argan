use std::{
	any::Any,
	convert::Infallible,
	fmt::Debug,
	future::{ready, IntoFuture},
	process::Output,
	sync::Arc,
};

use bytes::Bytes;
use http::{Extensions, Method, StatusCode, Uri};
use http_body_util::BodyExt;
use percent_encoding::percent_decode_str;

use crate::{
	body::{Body, HttpBody},
	common::{mark::Private, BoxedError, BoxedFuture, MaybeBoxed, Uncloneable, SCOPE_VALIDITY},
	handler::{
		futures::ResponseToResultFuture,
		request_handlers::{
			self, handle_mistargeted_request, handle_unimplemented_method, MethodHandlers,
			MistargetedRequestHandler, UnimplementedMethodHandler,
		},
		AdaptiveHandler, Args, BoxedHandler, Handler, IntoHandler, Service,
	},
	middleware::{BoxedLayer, Layer, ResponseFutureBoxer},
	pattern::{ParamsList, Pattern},
	request::Request,
	response::{IntoResponse, Redirect, Response},
	routing::{self, RouteTraversal, RoutingState, UnusedRequest},
};

use super::{config::ConfigFlags, ResourceExtensions, ResourceLayerTarget};

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

#[derive(Clone)]
pub struct ResourceService {
	pattern: Pattern,
	extensions: Extensions,

	request_receiver: MaybeBoxed<RequestReceiver>,
	some_mistargeted_request_handler: Option<BoxedHandler>,
}

impl ResourceService {
	#[inline(always)]
	pub(super) fn new(
		pattern: Pattern,
		extensions: Extensions,
		request_receiver: MaybeBoxed<RequestReceiver>,
		some_mistargeted_request_handler: Option<BoxedHandler>,
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

	#[inline]
	pub(crate) fn handle<B>(&self, request: Request<B>, args: &mut Args) -> BoxedFuture<Response>
	where
		B: HttpBody<Data = Bytes> + Send + Sync + 'static,
		B::Error: Into<BoxedError>,
	{
		let mut request = request.map(Body::new);

		// TODO: Replace resource extensions in the args.

		match &self.request_receiver {
			MaybeBoxed::Boxed(boxed_request_receiver) => boxed_request_receiver.handle(request, args),
			MaybeBoxed::Unboxed(request_receiver) => request_receiver.handle(request, args),
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
	type Future = ResponseToResultFuture<BoxedFuture<Response>>;

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
					return ResponseToResultFuture::from(Box::pin(ready(
						StatusCode::BAD_REQUEST.into_response(),
					)) as BoxedFuture<Response>); // ???
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
			resource_extensions: ResourceExtensions::new_borrowed(&self.extensions),
			handler_extension: &(),
		};

		if matched {
			ResponseToResultFuture::from(match &self.request_receiver {
				MaybeBoxed::Boxed(boxed_request_receiver) => {
					boxed_request_receiver.handle(request, &mut args)
				}
				MaybeBoxed::Unboxed(request_receiver) => request_receiver.handle(request, &mut args),
			})
		} else {
			ResponseToResultFuture::from(handle_mistargeted_request(
				request,
				args.routing_state,
				self
					.some_mistargeted_request_handler
					.as_ref()
					.map(|handler| (handler, args.resource_extensions)),
			))
		}
	}
}

// --------------------------------------------------

#[derive(Clone)]
pub(crate) struct RequestReceiver {
	some_request_passer: Option<MaybeBoxed<RequestPasser>>,
	some_request_handler: Option<Arc<MaybeBoxed<RequestHandler>>>,
	some_mistargeted_request_handler: Option<BoxedHandler>,

	config_flags: ConfigFlags,
}

impl RequestReceiver {
	pub(crate) fn new(
		some_request_passer: Option<MaybeBoxed<RequestPasser>>,
		some_request_handler: Option<Arc<MaybeBoxed<RequestHandler>>>,
		some_mistargeted_request_handler: Option<BoxedHandler>,
		config_flags: ConfigFlags,
		middleware: Vec<ResourceLayerTarget>,
	) -> MaybeBoxed<Self> {
		let request_receiver = Self {
			some_request_passer,
			some_request_handler,
			some_mistargeted_request_handler,
			config_flags,
		};

		let mut maybe_boxed_request_receiver = MaybeBoxed::Unboxed(request_receiver);

		for layer in middleware {
			use super::layer_targets::ResourceLayerTargetValue;

			if let ResourceLayerTargetValue::RequestReceiver(boxed_layer) = layer.0 {
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
	type Future = BoxedFuture<Response>;

	#[inline]
	fn handle(&self, mut request: Request, args: &mut Args) -> Self::Future {
		if args
			.routing_state
			.path_traversal
			.has_remaining_segments(request.uri().path())
		{
			let resource_is_subtree_handler = self.is_subtree_hander();

			if let Some(request_passer) = self.some_request_passer.as_ref() {
				if resource_is_subtree_handler {
					args.routing_state.subtree_handler_exists = true;
				}

				let next_segment_index = args.routing_state.path_traversal.next_segment_index();

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

				let resource_extensions = args.resource_extensions.clone().into_owned();

				return Box::pin(async move {
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

					let mut routing_state = response
						.extensions_mut()
						.remove::<Uncloneable<RoutingState>>()
						.expect("Uncloneable<RoutingState> should always exist when unused request is returned")
						.into_inner()
						.expect("RoutingState should always exist in Uncloneable");

					routing_state
						.path_traversal
						.revert_to_segment(next_segment_index);

					let mut args = Args {
						routing_state,
						resource_extensions,
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
				let resource_extensions = args.resource_extensions.take();

				return handle_mistargeted_request(
					request,
					routing_state,
					self
						.some_mistargeted_request_handler
						.as_ref()
						.map(|handler| (handler, resource_extensions)),
				);
			}
		}

		if let Some(request_handler) = self.some_request_handler.as_ref() {
			let request_path_ends_with_slash = args
				.routing_state
				.path_traversal
				.ends_with_slash(request.uri().path());

			let resource_path_ends_with_slash = self.config_flags.has(ConfigFlags::ENDS_WITH_SLASH);

			let handle = if request_path_ends_with_slash && !resource_path_ends_with_slash {
				if self
					.config_flags
					.has(ConfigFlags::REDIRECTS_ON_UNMATCHING_SLASH)
				{
					let path = request.uri().path();

					return Box::pin(ready(
						Redirect::permanently(&path[..path.len() - 1]).into_response(),
					));
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

					return Box::pin(ready(Redirect::permanently(new_path).into_response()));
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
		let resource_extensions = args.resource_extensions.take();

		handle_mistargeted_request(
			request,
			routing_state,
			self
				.some_mistargeted_request_handler
				.as_ref()
				.map(|handler| (handler, resource_extensions)),
		)
	}
}

// ----------

#[derive(Clone)]
pub(crate) struct RequestPasser {
	some_static_resources: Option<Arc<[ResourceService]>>,
	some_regex_resources: Option<Arc<[ResourceService]>>,
	some_wildcard_resource: Option<Arc<ResourceService>>,

	some_mistargeted_request_handler: Option<BoxedHandler>,
}

impl RequestPasser {
	pub(crate) fn new(
		some_static_resources: Option<Arc<[ResourceService]>>,
		some_regex_resources: Option<Arc<[ResourceService]>>,
		some_wildcard_resource: Option<Arc<ResourceService>>,
		some_mistargeted_request_handler: Option<BoxedHandler>,
		middleware: &mut Vec<ResourceLayerTarget>,
	) -> MaybeBoxed<Self> {
		let request_passer = Self {
			some_static_resources,
			some_regex_resources,
			some_wildcard_resource,
			some_mistargeted_request_handler,
		};

		let mut maybe_boxed_request_passer = MaybeBoxed::Unboxed(request_passer);

		for layer in middleware.iter_mut().rev() {
			use super::layer_targets::ResourceLayerTargetValue;

			match layer.0 {
				ResourceLayerTargetValue::RequestPasser(_) => {
					let ResourceLayerTargetValue::RequestPasser(boxed_layer) = layer.0.take() else {
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
	type Future = BoxedFuture<Response>;

	#[inline]
	fn handle(&self, mut request: Request, args: &mut Args) -> Self::Future {
		let some_next_resource = 'some_next_resource: {
			let (next_segment, _) = args
				.routing_state
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

			let Ok(decoded_segment) = percent_decode_str(next_segment).decode_utf8() else {
				return Box::pin(ready(StatusCode::BAD_REQUEST.into_response()));
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
			match &next_resource.request_receiver {
				MaybeBoxed::Boxed(boxed_request_receiver) => {
					return boxed_request_receiver.handle(request, args);
				}
				MaybeBoxed::Unboxed(request_receiver) => {
					return request_receiver.handle(request, args);
				}
			}
		}

		let routing_state = std::mem::take(&mut args.routing_state);
		let resource_extensions = args.resource_extensions.take();

		handle_mistargeted_request(
			request,
			routing_state,
			self
				.some_mistargeted_request_handler
				.as_ref()
				.map(|handler| (handler, resource_extensions)),
		)
	}
}

// ----------

#[derive(Clone)]
pub(crate) struct RequestHandler {
	allowed_methods: String,

	method_handlers: Vec<(Method, BoxedHandler)>,
	some_wildcard_method_handler: Option<BoxedHandler>,
}

impl RequestHandler {
	pub(crate) fn new(
		method_handlers: MethodHandlers,
		middleware: &mut Vec<ResourceLayerTarget>,
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

		use super::layer_targets::ResourceLayerTargetValue;

		for layer in middleware.iter_mut().rev() {
			match layer.0 {
				ResourceLayerTargetValue::MethodHandler(..) => {
					let ResourceLayerTargetValue::MethodHandler(methods, boxed_layer) = layer.0.take() else {
						unreachable!()
					};

					for method in methods.into_iter().rev() {
						if let Err(method) = request_handler.wrap_method_handler(method, boxed_layer.clone()) {
							return Err(method);
						}
					}
				}
				ResourceLayerTargetValue::WildcardMethodHandler(_) => {
					let ResourceLayerTargetValue::WildcardMethodHandler(boxed_layer) = layer.0.take() else {
						unreachable!()
					};

					request_handler.wrap_wildcard_method_handler(boxed_layer);
				}
				ResourceLayerTargetValue::RequestHandler(_) => request_handler_middleware_exists = true,
				_ => {}
			}
		}

		if request_handler_middleware_exists {
			let mut boxed_request_handler = BoxedHandler::new(request_handler);

			for layer in middleware.iter_mut().rev() {
				if let ResourceLayerTargetValue::RequestHandler(_) = layer.0 {
					let ResourceLayerTargetValue::RequestHandler(boxed_layer) = layer.0.take() else {
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

	pub(crate) fn wrap_wildcard_method_handler(&mut self, boxed_layer: BoxedLayer) {
		let boxed_handler = match self.some_wildcard_method_handler.take() {
			Some(wildcard_method_handler) => wildcard_method_handler,
			None => {
				let allowed_methods = std::mem::take(&mut self.allowed_methods);
				let unimplemented_method_handler = UnimplementedMethodHandler::new(allowed_methods);

				BoxedHandler::new(ResponseFutureBoxer::wrap(unimplemented_method_handler))
			}
		};

		let boxed_handler = boxed_layer.wrap(boxed_handler.into());

		self.some_wildcard_method_handler.replace(boxed_handler);
	}
}

impl Handler for RequestHandler {
	type Response = Response;
	type Future = BoxedFuture<Response>;

	fn handle(&self, request: Request, args: &mut Args) -> Self::Future {
		let method = request.method().clone();
		let some_method_handler = self.method_handlers.iter().find(|(m, _)| m == method);

		if let Some((_, ref handler)) = some_method_handler {
			handler.handle(request, args).into()
		} else {
			if let Some(wildcard_method_handler) = self.some_wildcard_method_handler.as_ref() {
				wildcard_method_handler.handle(request, args)
			} else {
				handle_unimplemented_method(request, &self.allowed_methods)
			}
		}
	}
}

// --------------------------------------------------------------------------------

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
