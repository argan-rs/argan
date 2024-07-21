use std::{fmt::Debug, future::ready};

use argan_core::{
	response::{ErrorResponse, ResponseResult},
	BoxedFuture,
};
use http::{Extensions, Method, Uri};

use crate::{
	middleware::{targets::LayerTarget, BoxedLayer, Layer},
	request::{routing::NotAllowedMethodError, RequestContext},
	resource::{NotFoundResourceError, Resource},
	response::{BoxedErrorResponse, Response},
};

use super::{AdaptiveHandler, ArcHandler, Args, BoxedHandler, FinalHandler, Handler};

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

// --------------------------------------------------
// MethodHandlers

#[derive(Clone)]
pub(crate) struct MethodHandlers {
	pub(crate) method_handlers_list: Vec<(Method, BoxedHandler)>,
	pub(crate) wildcard_method_handler: WildcardMethodHandler,

	pub(crate) supported_methods: String,
}

impl MethodHandlers {
	pub(crate) fn new() -> Self {
		MethodHandlers {
			method_handlers_list: Vec::new(),
			wildcard_method_handler: WildcardMethodHandler::Default,

			supported_methods: String::new(),
		}
	}

	// ----------

	#[inline(always)]
	pub(crate) fn count(&self) -> usize {
		self.method_handlers_list.len()
	}

	#[inline(always)]
	pub(crate) fn has_some_effect(&self) -> bool {
		!self.method_handlers_list.is_empty() || self.has_custom_wildcard_method_handler()
	}

	#[inline(always)]
	pub(crate) fn has_custom_wildcard_method_handler(&self) -> bool {
		self.wildcard_method_handler.is_custom()
	}

	#[inline]
	pub(crate) fn set_handler(&mut self, method: Method, handler: BoxedHandler) {
		if self.method_handlers_list.iter().any(|(m, _)| m == method) {
			panic!("\"{}\" handler already exists", method)
		}

		if !self.supported_methods.is_empty() {
			self.supported_methods.push_str(", ");
		}

		self.supported_methods.push_str(method.as_str());
		self.method_handlers_list.push((method, handler));
	}

	#[inline(always)]
	pub(crate) fn set_wildcard_method_handler(&mut self, some_boxed_handler: Option<BoxedHandler>) {
		if self.has_custom_wildcard_method_handler() {
			panic!("wildcard method handler already exists")
		}

		if self.wildcard_method_handler.is_none() {
			panic!("wildcard method handler has been forbidden")
		}

		if let Some(boxed_handler) = some_boxed_handler {
			self.wildcard_method_handler = WildcardMethodHandler::Custom(boxed_handler);
		} else {
			self.wildcard_method_handler = WildcardMethodHandler::None(None);
			// The mistargeted request handler must be set when into_service() is called.
		}
	}
}

impl Debug for MethodHandlers {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(
			f,
			"MethodHandlers {{
				method_handlers count: {},
				wildcard_method_handler exists: {},
				supported methods: {}
			}}",
			self.method_handlers_list.len(),
			self.wildcard_method_handler.is_custom(),
			self.supported_methods.as_str(),
		)
	}
}

// --------------------------------------------------
// WildcardMethodHandler

#[derive(Default, Clone)]
pub(crate) enum WildcardMethodHandler {
	#[default]
	Default,
	Custom(BoxedHandler),
	None(Option<ArcHandler>), // Mistargeted request handler.
}

impl WildcardMethodHandler {
	pub(crate) fn is_default(&self) -> bool {
		matches!(self, Self::Default)
	}

	pub(crate) fn is_custom(&self) -> bool {
		matches!(self, Self::Custom(_))
	}

	pub(crate) fn is_none(&self) -> bool {
		matches!(self, Self::None(_))
	}

	pub(crate) fn wrap(&mut self, boxed_layer: BoxedLayer) {
		let boxed_handler = match self {
			Self::Default => BoxedHandler::new(UnsupportedMethodHandler),
			Self::Custom(boxed_handler) => std::mem::take(boxed_handler),
			Self::None(_) => panic!("middleware was provided for a forbidden wildcard method handler"),
		};

		let boxed_handler = boxed_layer.wrap(boxed_handler.into());

		*self = Self::Custom(boxed_handler);
	}
}

impl From<BoxedHandler> for WildcardMethodHandler {
	fn from(boxed_handler: BoxedHandler) -> Self {
		Self::Custom(boxed_handler)
	}
}

impl From<ArcHandler> for WildcardMethodHandler {
	fn from(arc_handler: ArcHandler) -> Self {
		Self::None(Some(arc_handler))
	}
}

impl Handler for WildcardMethodHandler {
	type Response = Response;
	type Error = BoxedErrorResponse;
	type Future = BoxedFuture<Result<Self::Response, Self::Error>>;

	fn handle(&self, request_context: RequestContext, args: Args) -> Self::Future {
		match self {
			Self::Default => {
				let request = request_context.into_request();
				let (head, ..) = request.into_parts();

				handle_unsupported_method(head.uri, head.method, head.extensions)
			}
			Self::Custom(boxed_handler) => boxed_handler.handle(request_context, args),
			Self::None(some_mistargeted_request_handler) => handle_mistargeted_request(
				request_context,
				args,
				some_mistargeted_request_handler.as_ref(),
			),
		}
	}
}

// --------------------------------------------------
// ImplementedMethods

#[derive(Debug, Clone)]
pub(crate) struct SupportedMethods(Box<str>);

impl SupportedMethods {
	#[inline(always)]
	pub(crate) fn new(implemented_methods: String) -> Self {
		Self(implemented_methods.into())
	}
}

impl AsRef<str> for SupportedMethods {
	#[inline(always)]
	fn as_ref(&self) -> &str {
		&self.0
	}
}

// --------------------------------------------------
// UnimplementedMethodHandler

#[derive(Default, Clone)]
pub(crate) struct UnsupportedMethodHandler;

impl Handler for UnsupportedMethodHandler {
	type Response = Response;
	type Error = BoxedErrorResponse;
	type Future = BoxedFuture<Result<Self::Response, Self::Error>>;

	fn handle(&self, request_context: RequestContext, _args: Args) -> Self::Future {
		let request = request_context.into_request();
		let (head, ..) = request.into_parts();

		handle_unsupported_method(head.uri, head.method, head.extensions)
	}
}

// -------------------------

pub(crate) fn handle_unsupported_method(
	resource_uri: Uri,
	unsupported_method: Method,
	mut extensions: Extensions,
) -> BoxedFuture<Result<Response, BoxedErrorResponse>> {
	let supported_methods = extensions
		.remove::<SupportedMethods>()
		.expect("resource {} should have at least one supported method");

	let not_allowed_method_error = NotAllowedMethodError {
		resource_uri,
		unsupported_method,
		supported_methods: supported_methods.0,
	};

	Box::pin(ready(not_allowed_method_error.into_error_result()))
}

// --------------------------------------------------------------------------------
// MistargetedRequestHandler (404 Not Found)

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
	type Error = BoxedErrorResponse;
	type Future = BoxedFuture<Result<Self::Response, Self::Error>>;

	fn handle(&self, request_context: RequestContext, _args: Args) -> Self::Future {
		let request = request_context.into_request();
		let (head, _) = request.into_parts();

		Box::pin(ready(
			NotFoundResourceError::new(head.uri).into_error_result(),
		))
	}
}

// -------------------------

pub(crate) fn wrap_mistargeted_request_handler(
	mut some_mistargeted_request_handler: Option<BoxedHandler>,
	middleware: &mut [LayerTarget<Resource>],
) -> Option<BoxedHandler> {
	for layer in middleware.iter_mut().rev() {
		if let LayerTarget::MistargetedRequestHandler(_) = layer {
			let LayerTarget::MistargetedRequestHandler(boxed_layer) = layer.take() else {
				unreachable!()
			};

			if let Some(boxed_mistargeted_request_handler) = some_mistargeted_request_handler {
				some_mistargeted_request_handler =
					Some(boxed_layer.wrap(AdaptiveHandler::from(boxed_mistargeted_request_handler)));
			} else {
				let boxed_mistargeted_request_handler =
					MistargetedRequestHandler::new().into_boxed_handler();

				some_mistargeted_request_handler =
					Some(boxed_layer.wrap(AdaptiveHandler::from(boxed_mistargeted_request_handler)));
			}
		}
	}

	some_mistargeted_request_handler
}

// -------------------------

pub(crate) fn handle_mistargeted_request(
	request_context: RequestContext,
	args: Args,
	mut some_custom_handler_with_extensions: Option<&ArcHandler>,
) -> BoxedFuture<ResponseResult> {
	if let Some(mistargeted_request_handler) = some_custom_handler_with_extensions.take() {
		// Custom handler for mistargeted requests.
		return mistargeted_request_handler.handle(request_context, args);
	}

	if request_context.noted_subtree_handler() {
		return Box::pin(ready(
			NotFoundResourceError::new_with_request_context(request_context).into_error_result(),
		));
	}

	let request = request_context.into_request();
	let uri = request.into_parts().0.uri;

	Box::pin(ready(NotFoundResourceError::new(uri).into_error_result()))
}

// --------------------------------------------------------------------------------
