use std::{future::Future, marker::PhantomData};

use crate::{
	request::{FromRequest, FromRequestParts},
	response::{IntoResponse, Response},
	utils::{BoxedError, BoxedFuture, Either},
};

use super::*;

// --------------------------------------------------

struct HandlerFn<Func, M> {
	func: Func,
	_mark: PhantomData<fn() -> M>,
}

impl<Func, A> Clone for HandlerFn<Func, A>
where
	Func: Clone,
{
	fn clone(&self) -> Self {
		HandlerFn {
			func: self.func.clone(),
			_mark: PhantomData,
		}
	}
}

// ----------

impl<Func, A, RqB, Fut, R, E> Service<Request<RqB>> for HandlerFn<Func, (A, R)>
where
	Func: FnMut(A) -> Fut + Clone + 'static,
	A: FromRequest<RqB>,
	RqB: 'static,
	Fut: Future<Output = Result<R, E>>,
	R: IntoResponse,
	E: Into<BoxedError>,
{
	type Response = Response;
	type Error = BoxedError;
	type Future = BoxedFuture<Result<Self::Response, Self::Error>>;

	#[inline]
	fn call(&mut self, req: Request<RqB>) -> Self::Future {
		// TODO: Maybe we should clone on a call side or we should clone the func?
		let mut self_clone = self.clone();

		Box::pin(async move {
			let arg = match A::from_request(req) {
				Ok(v) => v,
				Err(Either::Left(v)) => return Ok(v.into_response()),
				Err(Either::Right(e)) => return Err(e.into()),
			};

			match (self_clone.func)(arg).await {
				Ok(v) => Ok(v.into_response()),
				Err(e) => Err(e.into()),
			}
		})
	}
}

impl<Func, A, RqB, Fut, R, E> IntoHandler<HandlerFn<Func, (A, R)>, RqB> for Func
where
	Func: FnMut(A) -> Fut + Clone + Send + 'static,
	A: FromRequest<RqB>,
	RqB: 'static,
	Fut: Future<Output = Result<R, E>>,
	R: IntoResponse,
	E: Into<BoxedError>,
{
	#[inline]
	fn into_handler(self) -> HandlerFn<Func, (A, R)> {
		HandlerFn {
			func: self,
			_mark: PhantomData,
		}
	}
}

// ----------

impl<Func, A1, LA, RqB, Fut, R, E> Service<Request<RqB>> for HandlerFn<Func, (A1, LA, R)>
where
	Func: FnMut(A1, LA) -> Fut + Clone + 'static,
	A1: FromRequestParts,
	LA: FromRequest<RqB>,
	RqB: 'static,
	Fut: Future<Output = Result<R, E>>,
	R: IntoResponse,
	E: Into<BoxedError>,
{
	type Response = Response;
	type Error = BoxedError;
	type Future = BoxedFuture<Result<Self::Response, Self::Error>>;

	#[inline]
	fn call(&mut self, req: Request<RqB>) -> Self::Future {
		let mut self_clone = self.clone();

		Box::pin(async move {
			let (parts, body) = req.into_parts();

			let arg1 = match A1::from_request_parts(&parts) {
				Ok(v) => v,
				Err(Either::Left(v)) => return Ok(v.into_response()),
				Err(Either::Right(e)) => return Err(e.into()),
			};

			let req = Request::<RqB>::from_parts(parts, body);

			let last_arg = match LA::from_request(req) {
				Ok(v) => v,
				Err(Either::Left(v)) => return Ok(v.into_response()),
				Err(Either::Right(e)) => return Err(e.into()),
			};

			match (self_clone.func)(arg1, last_arg).await {
				Ok(v) => Ok(v.into_response()),
				Err(e) => Err(e.into()),
			}
		})
	}
}

impl<Func, A1, LA, RqB, Fut, R, E> IntoHandler<HandlerFn<Func, (A1, LA, R)>, RqB> for Func
where
	Func: FnMut(A1, LA) -> Fut + Clone + Send + 'static,
	A1: FromRequestParts,
	LA: FromRequest<RqB>,
	RqB: 'static,
	Fut: Future<Output = Result<R, E>>,
	R: IntoResponse,
	E: Into<BoxedError>,
{
	#[inline]
	fn into_handler(self) -> HandlerFn<Func, (A1, LA, R)> {
		HandlerFn {
			func: self,
			_mark: PhantomData,
		}
	}
}

// --------------------------------------------------

#[cfg(test)]
mod test {
	use super::*;

	// --------------------------------------------------
	// --------------------------------------------------

	fn is_handler<H, RqB>(_: H)
	where
		H: Handler<RqB>,
	{
	}

	fn is_into_handler<IH, H, RqB>(h: IH)
	where
		IH: IntoHandler<H, RqB>,
		H: Handler<RqB>,
	{
		is_service(h.into_handler())
	}

	fn is_service<S, RqB>(_: S)
	where
		S: Service<Request<RqB>> + Clone + Send,
	{
	}

	// -------------------------

	async fn handler(_: Request) -> Result<Response, BoxedError> {
		todo!()
	}

	// --------------------------------------------------

	fn test_type() {
		is_into_handler(handler);

		let handler_fn = HandlerFn {
			func: handler,
			_mark: PhantomData,
		};
		is_handler(handler_fn);

		let handler_fn = HandlerFn {
			func: handler,
			_mark: PhantomData,
		};
		is_into_handler(handler_fn);
	}
}
