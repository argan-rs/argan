use hyper::header::{HeaderName, HeaderValue};

use crate::{
	request::Request,
	response::Response,
	routing::{Method, StatusCode, UnusedRequest},
	utils::{BoxedError, BoxedFuture},
};

use super::BoxedHandler;

// --------------------------------------------------------------------------------

pub(crate) struct MethodHandlers<B> {
	method_handlers: Vec<(Method, BoxedHandler<B>)>,
	unsupported_method_handler: Option<BoxedHandler<B>>,
}

impl<B> MethodHandlers<B> {
	pub(crate) fn new() -> MethodHandlers<B> {
		MethodHandlers {
			method_handlers: Vec::new(),
			unsupported_method_handler: None,
		}
	}

	#[inline]
	pub(crate) fn is_empty(&self) -> bool {
		self.method_handlers.is_empty()
	}

	#[inline]
	pub(crate) fn set_handler(&mut self, method: Method, handler: BoxedHandler<B>) {
		if self.method_handlers.iter().any(|(m, _)| m == method) {
			panic!("{} handler already exists", method)
		}

		self.method_handlers.push((method, handler));
	}

	pub(crate) fn allowed_methods(&self) -> AllowedMethods {
		let mut list = String::new();
		self
			.method_handlers
			.iter()
			.for_each(|(method, _)| list.push_str(method.as_str()));

		AllowedMethods(list)
	}

	#[inline]
	pub(crate) fn handle(&self, mut request: Request<B>) -> BoxedFuture<Result<Response, BoxedError>>
	where
		B: Send + Sync + 'static,
	{
		let method = request.method().clone();
		let some_handler = self
			.method_handlers
			.iter()
			.find(|(m, _)| m == method)
			.map(|(_, h)| h.clone());

		match some_handler {
			Some(mut handler) => handler.call(request),
			None => {
				let allowed_methods = self.allowed_methods();
				request.extensions_mut().insert(allowed_methods);

				match self.unsupported_method_handler.as_ref() {
					Some(mut not_allowed_method_handler) => not_allowed_method_handler.call(request),
					None => Box::pin(handle_not_allowed_method(request)),
				}
			}
		}
	}
}

// --------------------------------------------------

pub(crate) struct AllowedMethods(String);

#[inline]
async fn handle_not_allowed_method<RqB>(mut request: Request<RqB>) -> Result<Response, BoxedError> {
	let allowed_methods = request.extensions_mut().remove::<AllowedMethods>().unwrap();
	let allowed_methods_header_value = HeaderValue::from_str(&allowed_methods.0).unwrap();

	let mut response = Response::default();
	*response.status_mut() = StatusCode::METHOD_NOT_ALLOWED;
	response.headers_mut().append(
		HeaderName::from_static("Allow"),
		allowed_methods_header_value,
	);

	Ok(response)
}

// --------------------------------------------------

#[inline]
pub(crate) async fn misdirected_request_handler(request: Request) -> Result<Response, BoxedError> {
	let mut response = Response::default();
	*response.status_mut() = StatusCode::NOT_FOUND;
	response
		.extensions_mut()
		.insert(UnusedRequest::from(request));

	Ok(response)
}
