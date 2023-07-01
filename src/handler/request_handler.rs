use crate::{
	handler::{HandlerService, Service},
	request::Request,
	response::Response,
	routing::{Method, StatusCode, UnusedRequest},
	utils::{BoxedError, BoxedFuture},
};

// --------------------------------------------------------------------------------

pub(crate) struct RequestHandler<RqB> {
	get: Option<HandlerService<RqB>>,
	put: Option<HandlerService<RqB>>,
	post: Option<HandlerService<RqB>>,
	head: Option<HandlerService<RqB>>,
	patch: Option<HandlerService<RqB>>,
	trace: Option<HandlerService<RqB>>,
	delete: Option<HandlerService<RqB>>,
	options: Option<HandlerService<RqB>>,
	connect: Option<HandlerService<RqB>>,

	not_allowed_method: Option<HandlerService<RqB>>,
}

impl<RqB> RequestHandler<RqB> {
	fn set_handler(&mut self, method: Method, handler: HandlerService<RqB>) {
		match method {
			Method::GET => {
				if self.get.is_some() {
					panic!("{} handler exists", method)
				}

				self.get = Some(handler);
			}
			Method::POST => {
				if self.post.is_some() {
					panic!("{} handler exists", method)
				}

				self.post = Some(handler);
			}
			Method::PUT => {
				if self.put.is_some() {
					panic!("{} handler exists", method)
				}

				self.put = Some(handler);
			}
			Method::DELETE => {
				if self.delete.is_some() {
					panic!("{} handler exists", method)
				}

				self.delete = Some(handler);
			}
			Method::HEAD => {
				if self.head.is_some() {
					panic!("{} handler exists", method)
				}

				self.head = Some(handler);
			}
			Method::OPTIONS => {
				if self.options.is_some() {
					panic!("{} handler exists", method)
				}

				self.options = Some(handler);
			}
			Method::CONNECT => {
				if self.connect.is_some() {
					panic!("{} handler exists", method)
				}

				self.connect = Some(handler);
			}
			Method::PATCH => {
				if self.patch.is_some() {
					panic!("{} handler exists", method)
				}

				self.patch = Some(handler);
			}
			Method::TRACE => {
				if self.trace.is_some() {
					panic!("{} handler exists", method)
				}

				self.trace = Some(handler);
			}
			_ => {
				todo!()
			}
		};
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
		let some_handler = match method {
			Method::GET => self.get.as_ref().cloned(),
			Method::POST => self.post.as_ref().cloned(),
			Method::PUT => self.put.as_ref().cloned(),
			Method::DELETE => self.delete.as_ref().cloned(),
			Method::HEAD => self.head.as_ref().cloned(),
			Method::OPTIONS => self.options.as_ref().cloned(),
			Method::CONNECT => self.connect.as_ref().cloned(),
			Method::PATCH => self.patch.as_ref().cloned(),
			Method::TRACE => self.trace.as_ref().cloned(),
			_ => {
				todo!()
			}
		};

		match some_handler {
			Some(mut handler) => handler.call(request),
			None => match self.not_allowed_method.as_ref().cloned() {
				Some(mut not_allowed_method_handler) => not_allowed_method_handler.call(request),
				None => Box::pin(handle_not_allowed_method(request)),
			},
		}
	}
}

// TODO: Must provide allowed methods of the RequestHandler.
async fn handle_not_allowed_method<RqB>(request: Request<RqB>) -> Result<Response, BoxedError> {
	todo!()
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
