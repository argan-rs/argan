use hyper::header::{HeaderName, HeaderValue};

use crate::{
	body::IncomingBody,
	middleware::{IntoResponseAdapter, Layer, RequestBodyAdapter, ResponseFutureBoxer},
	request::Request,
	response::{IntoResponse, Response},
	routing::{Method, StatusCode, UnusedRequest},
	utils::BoxedFuture,
};

use super::{AdaptiveHandler, BoxedHandler, Handler};

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

pub(crate) struct MethodHandlers {
	method_handlers: Vec<(Method, BoxedHandler)>,
	unsupported_method_handler: Option<BoxedHandler>,
}

impl MethodHandlers {
	pub(crate) fn new() -> MethodHandlers {
		MethodHandlers {
			method_handlers: Vec::new(),
			unsupported_method_handler: None,
		}
	}

	// ----------

	#[inline]
	pub(crate) fn is_empty(&self) -> bool {
		self.method_handlers.is_empty()
	}

	#[inline]
	pub(crate) fn set_handler(&mut self, method: Method, handler: BoxedHandler) {
		if self.method_handlers.iter().any(|(m, _)| m == method) {
			panic!("{} handler already exists", method)
		}

		self.method_handlers.push((method, handler));
	}

	#[inline]
	pub(crate) fn wrap_handler<L, LayeredB>(&mut self, method: Method, layer: L)
	where
		L: Layer<AdaptiveHandler<LayeredB>, LayeredB>,
		L::Handler: Handler<IncomingBody> + Sync + 'static,
		<L::Handler as Handler<IncomingBody>>::Response: IntoResponse,
	{
		let Some(position) = self.method_handlers.iter().position(|(m, _)| m == method) else {
			panic!("{} handler doesn't exists", method)
		};

		let (method, boxed_handler) = std::mem::take(&mut self.method_handlers[position]);
		let adaptive_handler = AdaptiveHandler::from(RequestBodyAdapter::wrap(boxed_handler));
		let layered_handler = layer.wrap(adaptive_handler);
		let ready_handler = ResponseFutureBoxer::wrap(IntoResponseAdapter::wrap(layered_handler));
		let handler = ready_handler.into_boxed_handler();

		self.method_handlers[position] = (method, handler);
	}

	#[inline]
	pub(crate) fn allowed_methods(&self) -> AllowedMethods {
		let mut list = String::new();
		self
			.method_handlers
			.iter()
			.for_each(|(method, _)| list.push_str(method.as_str()));

		AllowedMethods(list)
	}

	// ----------

	#[inline]
	pub(crate) fn handle(&self, mut request: Request<IncomingBody>) -> BoxedFuture<Response> {
		let method = request.method().clone();
		let some_handler = self
			.method_handlers
			.iter()
			.find(|(m, _)| m == method)
			.map(|(_, h)| h.clone());

		match some_handler {
			Some(mut handler) => handler.handle(request),
			None => {
				let allowed_methods = self.allowed_methods();
				request.extensions_mut().insert(allowed_methods);

				match self.unsupported_method_handler.as_ref() {
					Some(mut not_allowed_method_handler) => not_allowed_method_handler.handle(request),
					None => Box::pin(handle_not_allowed_method(request)),
				}
			}
		}
	}
}

// --------------------------------------------------

pub(crate) struct AllowedMethods(String);

#[inline]
async fn handle_not_allowed_method(mut request: Request<IncomingBody>) -> Response {
	let allowed_methods = request.extensions_mut().remove::<AllowedMethods>().unwrap();
	let allowed_methods_header_value = HeaderValue::from_str(&allowed_methods.0).unwrap();

	let mut response = Response::default();
	*response.status_mut() = StatusCode::METHOD_NOT_ALLOWED;
	response.headers_mut().append(
		HeaderName::from_static("Allow"),
		allowed_methods_header_value,
	);

	response
}

#[inline]
pub(crate) async fn misdirected_request_handler(request: Request<IncomingBody>) -> Response {
	let mut response = Response::default();
	*response.status_mut() = StatusCode::NOT_FOUND;
	response
		.extensions_mut()
		.insert(UnusedRequest::from(request));

	response
}
