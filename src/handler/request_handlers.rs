use std::{
	fmt::Debug,
	future::{ready, Ready},
};

use http::{Extensions, HeaderName, HeaderValue, Method, StatusCode};

use crate::{
	common::{mark::Private, BoxedFuture, Uncloneable},
	middleware::{BoxedLayer, LayerTarget, ResponseFutureBoxer},
	request::Request,
	resource::ResourceExtensions,
	response::{IntoResponse, Response},
	routing::{RoutingState, UnusedRequest},
};

use super::{AdaptiveHandler, Args, BoxedHandler, FinalHandler, Handler, IntoHandler};

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

#[derive(Clone)]
pub(crate) struct MethodHandlers {
	pub(crate) method_handlers: Vec<(Method, BoxedHandler)>,
	pub(crate) some_wildcard_method_handler: Option<BoxedHandler>,

	pub(crate) send_allowed_methods: bool,
	pub(crate) allowed_methods: String,
}

impl MethodHandlers {
	pub(crate) fn new() -> Self {
		MethodHandlers {
			method_handlers: Vec::new(),
			some_wildcard_method_handler: None,

			send_allowed_methods: true,
			allowed_methods: String::new(),
		}
	}

	// ----------

	#[inline(always)]
	pub(crate) fn count(&self) -> usize {
		self.method_handlers.len()
	}

	#[inline(always)]
	pub(crate) fn is_empty(&self) -> bool {
		self.method_handlers.is_empty() && self.some_wildcard_method_handler.is_none()
	}

	#[inline(always)]
	pub(crate) fn has_some_effect(&self) -> bool {
		!self.method_handlers.is_empty() || self.some_wildcard_method_handler.is_some()
	}

	#[inline(always)]
	pub(crate) fn has_wildcard_method_handler(&self) -> bool {
		self.some_wildcard_method_handler.is_some()
	}

	#[inline(always)]
	pub(crate) fn no_allowed_methods(&mut self) {
		self.send_allowed_methods = false;
		self.allowed_methods = String::new();
	}

	#[inline]
	pub(crate) fn set_handler(&mut self, method: Method, handler: BoxedHandler) {
		if self.method_handlers.iter().any(|(m, _)| m == method) {
			panic!("{} handler already exists", method)
		}

		if self.send_allowed_methods && !self.allowed_methods.is_empty() {
			self.allowed_methods.push_str(", ");
		}

		if self.send_allowed_methods {
			self.allowed_methods.push_str(method.as_str());
		}

		self.method_handlers.push((method, handler));
	}

	#[inline(always)]
	pub(crate) fn set_wildcard_method_handler(&mut self, handler: BoxedHandler) {
		self.some_wildcard_method_handler = Some(handler);
	}
}

impl Debug for MethodHandlers {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(
			f,
			"MethodHandlers {{ method_handlers count: {}, all_methods_handler exists: {} }}",
			self.method_handlers.len(),
			self.some_wildcard_method_handler.is_some(),
		)
	}
}

// --------------------------------------------------

#[derive(Default, Clone)]
pub(crate) struct UnimplementedMethodHandler(String);

impl UnimplementedMethodHandler {
	#[inline(always)]
	pub(crate) fn new(allowed_methods: String) -> Self {
		Self(allowed_methods)
	}
}

impl Handler for UnimplementedMethodHandler {
	type Response = Response;
	type Future = Ready<Response>;

	fn handle(&self, request: Request, _args: &mut Args) -> Self::Future {
		match HeaderValue::from_str(&self.0) {
			Ok(header_value) => {
				let mut response = Response::default();
				*response.status_mut() = StatusCode::METHOD_NOT_ALLOWED;
				response
					.headers_mut()
					.append(HeaderName::from_static("Allow"), header_value);

				ready(response)
			}
			Err(_) => ready(StatusCode::INTERNAL_SERVER_ERROR.into_response()),
		}
	}
}

// -------------------------

pub(crate) fn handle_unimplemented_method(
	mut request: Request,
	allowed_methods: &str,
) -> BoxedFuture<Response> {
	let mut response = Response::default();
	*response.status_mut() = StatusCode::METHOD_NOT_ALLOWED;

	if allowed_methods.is_empty() {
		return Box::pin(ready(response));
	}

	match HeaderValue::from_str(allowed_methods) {
		Ok(header_value) => {
			response
				.headers_mut()
				.append(HeaderName::from_static("Allow"), header_value);
		}
		Err(_) => {
			*response.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;
		}
	}

	Box::pin(ready(response))
}

// --------------------------------------------------------------------------------
// Mistargeted Request Handler (404 Not Found)

#[derive(Default, Clone)]
pub(crate) struct MistargetedRequestHandler;

impl MistargetedRequestHandler {
	#[inline(always)]
	pub(crate) fn new() -> Self {
		Self
	}
}

impl Handler for MistargetedRequestHandler {
	type Response = Response;
	type Future = Ready<Response>;

	fn handle(&self, _request: Request, _args: &mut Args) -> Self::Future {
		let mut response = Response::default();
		*response.status_mut() = StatusCode::NOT_FOUND;

		ready(response)
	}
}

// -------------------------

pub(crate) fn wrap_mistargeted_request_handler(
	mut some_mistargeted_request_handler: Option<BoxedHandler>,
	middleware: &mut Vec<LayerTarget>,
) -> Option<BoxedHandler> {
	use crate::middleware::Inner;

	for layer in middleware.iter_mut().rev() {
		if let Inner::MistargetedRequestHandler(_) = &mut layer.0 {
			let Inner::MistargetedRequestHandler(boxed_layer) = layer.0.take() else {
				unreachable!()
			};

			if let Some(boxed_mistargeted_request_handler) = some_mistargeted_request_handler {
				some_mistargeted_request_handler =
					Some(boxed_layer.wrap(AdaptiveHandler::from(boxed_mistargeted_request_handler)));
			} else {
				let boxed_mistargeted_request_handler =
					ResponseFutureBoxer::wrap(MistargetedRequestHandler::new()).into_boxed_handler();

				some_mistargeted_request_handler =
					Some(boxed_layer.wrap(AdaptiveHandler::from(boxed_mistargeted_request_handler)));
			}
		}
	}

	some_mistargeted_request_handler
}

// -------------------------

pub(crate) fn handle_mistargeted_request(
	mut request: Request,
	routing_state: RoutingState,
	mut some_custom_handler_with_extensions: Option<(&BoxedHandler, ResourceExtensions)>,
) -> BoxedFuture<Response> {
	if let Some((mistargeted_request_handler, resource_extensions)) =
		some_custom_handler_with_extensions.take()
	{
		// request
		// 	.extensions_mut()
		// 	.insert(Uncloneable::from(routing_state));

		let mut args = Args {
			routing_state,
			resource_extensions,
			handler_extension: &(),
		};
		// Args::with_resource_extensions(resource_extensions);

		// Custom handler with a custom 404 Not Found respnose.
		return mistargeted_request_handler.handle(request, &mut args);
	}

	let mut response = Response::default();
	*response.status_mut() = StatusCode::NOT_FOUND;

	if routing_state.subtree_handler_exists {
		response
			.extensions_mut()
			.insert(Uncloneable::from(UnusedRequest::from(request)));
	}

	Box::pin(ready(response))
}

// --------------------------------------------------------------------------------
