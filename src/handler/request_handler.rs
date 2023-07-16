use hyper::header::{HeaderName, HeaderValue};

use crate::{
	handler::{HandlerService, Service},
	request::Request,
	response::Response,
	routing::{Method, StatusCode, UnusedRequest},
	utils::{BoxedError, BoxedFuture},
};

// --------------------------------------------------------------------------------

pub(crate) struct RequestHandler<RqB> {
	method_handlers: Vec<(Method, HandlerService<RqB>)>,
	not_allowed_method: Option<HandlerService<RqB>>,
}

impl<RqB> RequestHandler<RqB> {
	fn set_handler(&mut self, method: Method, handler: HandlerService<RqB>) {
		if self
			.method_handlers
			.iter()
			.find(|(m, _)| m == method)
			.is_some()
		{
			panic!("{} handler already exists", method)
		}

		self.method_handlers.push((method, handler));
	}
}

impl<RqB> Service<Request<RqB>> for RequestHandler<RqB>
where
	Self: 'static,
{
	type Response = Response;
	type Error = BoxedError;
	type Future = BoxedFuture<Result<Response, BoxedError>>;

	fn call(&mut self, request: Request<RqB>) -> Self::Future {
		let method = request.method().clone();
		let some_handler = self
			.method_handlers
			.iter()
			.find(|(m, _)| m == method)
			.map(|(_, h)| h.clone());

		match some_handler {
			Some(mut handler) => handler.call(request),
			None => match self.not_allowed_method.as_ref().cloned() {
				Some(mut not_allowed_method_handler) => not_allowed_method_handler.call(request),
				None => Box::pin(handle_not_allowed_method(request)),
			},
		}
	}
}

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
