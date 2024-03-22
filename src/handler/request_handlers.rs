use std::{
	fmt::Debug,
	future::{ready, Ready},
};

use http::{header::InvalidHeaderValue, Extensions, HeaderName, HeaderValue, Method, StatusCode};

use crate::{
	common::{mark::Private, BoxedError, BoxedFuture, Uncloneable},
	data::extensions::NodeExtensions,
	middleware::{layer_targets::LayerTarget, BoxedLayer, Layer, ResponseResultFutureBoxer},
	request::Request,
	resource::Resource,
	response::{BoxedErrorResponse, IntoResponse, Response, ResponseError},
	routing::{RoutingState, UnusedRequest},
};

use super::{AdaptiveHandler, ArcHandler, Args, BoxedHandler, FinalHandler, Handler, IntoHandler};

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

#[derive(Clone)]
pub(crate) struct MethodHandlers {
	pub(crate) method_handlers_list: Vec<(Method, BoxedHandler)>,
	pub(crate) wildcard_method_handler: WildcardMethodHandler,

	pub(crate) implemented_methods: String,
}

impl MethodHandlers {
	pub(crate) fn new() -> Self {
		MethodHandlers {
			method_handlers_list: Vec::new(),
			wildcard_method_handler: WildcardMethodHandler::Default,

			implemented_methods: String::new(),
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
			panic!("{} handler already exists", method)
		}

		if !self.implemented_methods.is_empty() {
			self.implemented_methods.push_str(", ");
		}

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
			"MethodHandlers {{ method_handlers count: {}, wildcard_method_handler exists: {} }}",
			self.method_handlers_list.len(),
			self.wildcard_method_handler.is_custom(),
		)
	}
}

// --------------------------------------------------

#[derive(Default, Clone)]
pub(crate) enum WildcardMethodHandler {
	#[default]
	Default,
	Custom(BoxedHandler),
	None(Option<ArcHandler>), // Mistargeted request handler.
}

impl WildcardMethodHandler {
	pub(crate) fn is_default(&self) -> bool {
		if let Self::Default = self {
			true
		} else {
			false
		}
	}

	pub(crate) fn is_custom(&self) -> bool {
		if let Self::Custom(_) = self {
			true
		} else {
			false
		}
	}

	pub(crate) fn is_none(&self) -> bool {
		if let Self::None(_) = self {
			true
		} else {
			false
		}
	}

	pub(crate) fn wrap(&mut self, boxed_layer: BoxedLayer) {
		let boxed_handler = match self {
			Self::Default => {
				BoxedHandler::new(ResponseResultFutureBoxer::wrap(UnimplementedMethodHandler))
			}
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

	fn handle(&self, request: Request, args: &mut Args) -> Self::Future {
		match self {
			Self::Default => handle_unimplemented_method(args),
			Self::Custom(boxed_handler) => boxed_handler.handle(request, args),
			Self::None(some_mistargeted_request_handler) => {
				let routing_state = std::mem::take(&mut args.routing_state);
				let node_extensions = args.node_extensions.take();

				handle_mistargeted_request(
					request,
					routing_state,
					some_mistargeted_request_handler
						.as_ref()
						.map(|handler| (handler, node_extensions)),
				)
			}
		}
	}
}

// --------------------------------------------------

#[derive(Debug, Clone)]
pub(crate) struct ImplementedMethods(String);

impl ImplementedMethods {
	#[inline(always)]
	pub(crate) fn new(implemented_methods: String) -> Self {
		Self(implemented_methods)
	}
}

impl AsRef<str> for ImplementedMethods {
	#[inline(always)]
	fn as_ref(&self) -> &str {
		&self.0
	}
}

// --------------------------------------------------

#[derive(Default, Clone)]
pub(crate) struct UnimplementedMethodHandler;

impl Handler for UnimplementedMethodHandler {
	type Response = Response;
	type Error = BoxedErrorResponse;
	type Future = BoxedFuture<Result<Self::Response, Self::Error>>;

	fn handle(&self, request: Request, args: &mut Args) -> Self::Future {
		handle_unimplemented_method(args)
	}
}

// -------------------------

pub(crate) fn handle_unimplemented_method(
	args: &mut Args,
) -> BoxedFuture<Result<Response, BoxedErrorResponse>> {
	let mut response = StatusCode::METHOD_NOT_ALLOWED.into_response();

	let Some(implemented_methods) = args.node_extensions.get_ref::<ImplementedMethods>() else {
		return Box::pin(ready(Ok(response)));
	};

	match HeaderValue::from_str(implemented_methods.as_ref()) {
		Ok(header_value) => {
			response
				.headers_mut()
				.append(HeaderName::from_static("Allow"), header_value);
		}
		Err(error) => return Box::pin(ready(Err(AllowHeaderError::from(error).into()))),
	}

	Box::pin(ready(Ok(response)))
}

// -------------------------
// AllowHeaderError

#[derive(Debug, crate::ImplError)]
#[error("invalid allow header '{0}'")]
pub struct AllowHeaderError(#[from] InvalidHeaderValue);

impl IntoResponse for AllowHeaderError {
	fn into_response(self) -> Response {
		StatusCode::INTERNAL_SERVER_ERROR.into_response()
	}
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
	type Error = BoxedErrorResponse;
	type Future = Ready<Result<Self::Response, Self::Error>>;

	fn handle(&self, _request: Request, _args: &mut Args) -> Self::Future {
		let mut response = Response::default();
		*response.status_mut() = StatusCode::NOT_FOUND;

		ready(Ok(response))
	}
}

// -------------------------

pub(crate) fn wrap_mistargeted_request_handler(
	mut some_mistargeted_request_handler: Option<BoxedHandler>,
	middleware: &mut Vec<LayerTarget<Resource>>,
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
					ResponseResultFutureBoxer::wrap(MistargetedRequestHandler::new()).into_boxed_handler();

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
	mut some_custom_handler_with_extensions: Option<(&ArcHandler, NodeExtensions)>,
) -> BoxedFuture<Result<Response, BoxedErrorResponse>> {
	if let Some((mistargeted_request_handler, node_extensions)) =
		some_custom_handler_with_extensions.take()
	{
		let mut args = Args {
			routing_state,
			node_extensions,
			handler_extension: &(),
		};

		// Custom handler with a custom 404 Not Found respnose.
		return mistargeted_request_handler.handle(request, &mut args);
	}

	let mut response = StatusCode::NOT_FOUND.into_response();

	if routing_state.subtree_handler_exists {
		response
			.extensions_mut()
			.insert(Uncloneable::from(UnusedRequest::from(request)));

		response
			.extensions_mut()
			.insert(Uncloneable::from(routing_state));
	}

	Box::pin(ready(Ok(response)))
}

// --------------------------------------------------------------------------------
