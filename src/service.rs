use std::future::Future;

use hyper::service::Service;

use super::utils::*;

// --------------------------------------------------

pub trait IntoService<S, Req>
where
	S: Service<Req>,
{
	fn into_service(self) -> S;
}

// -------------------------

impl<S, Req> IntoService<S, Req> for S
where
	S: Service<Req>,
{
	fn into_service(self) -> S {
		self
	}
}

impl<Func, Fut, Req, Res, Err> IntoService<ServiceFn<Func>, Req> for Func
where
	Func: FnMut(Req) -> Fut,
	Fut: Future<Output = Result<Res, Err>>,
	Err: Into<BoxedError>,
{
	#[inline]
	fn into_service(self) -> ServiceFn<Func> {
		ServiceFn { func: self }
	}
}

// --------------------------------------------------

#[derive(Clone)]
pub struct ServiceFn<Func> {
	func: Func,
}

impl<Func, Fut, Req, Res, Err> Service<Req> for ServiceFn<Func>
where
	Func: FnMut(Req) -> Fut,
	Fut: Future<Output = Result<Res, Err>>,
{
	type Response = Res;
	type Error = Err;
	type Future = Fut;

	#[inline]
	fn call(&self, req: Req) -> Self::Future {
		(self.func)(req)
	}
}

impl<Func> std::convert::From<Func> for ServiceFn<Func> {
	#[inline]
	fn from(value: Func) -> Self {
		Self { func: value }
	}
}

// --------------------------------------------------

#[cfg(test)]
mod test {
	use std::pin::Pin;

	use crate::{request::Request, response::Response};

	use super::*;

	// -------------------------

	fn is_service<T, S, Req>(_: T)
	where
		T: IntoService<S, Req>,
		S: Service<Req>,
	{
	}

	// -------------------------

	async fn handler(_: Request<()>) -> Result<Response<()>, BoxedError> {
		todo!()
	}

	async fn handler_mut(mut _unused: Request<()>) -> Result<Response<()>, BoxedError> {
		todo!()
	}

	fn handler_returning_future(
		_: Request<()>,
	) -> Pin<Box<dyn Future<Output = Result<Response<()>, BoxedError>>>> {
		todo!()
	}

	// -------------------------

	#[test]
	fn test() {
		is_service(handler);
		is_service(handler_mut);
		is_service(handler_returning_future);

		let boxed_service = Box::new(handler);
		is_service(boxed_service);
	}
}
