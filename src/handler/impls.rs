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

// impl<Func, A> Clone for HandlerFn<Func, A>
// where
// 	Func: Clone,
// {
// 	#[inline]
// 	fn clone(&self) -> Self {
// 		HandlerFn {
// 			func: self.func.clone(),
// 			_mark: PhantomData,
// 		}
// 	}
// }

// ----------

impl<Func, RqB, Fut, E> Service<Request<RqB>> for HandlerFn<Func, Request<RqB>>
where
	Func: Fn(Request<RqB>) -> Fut,
	Fut: Future<Output = Result<Response, E>>,
	E: Into<BoxedError>,
{
	type Response = Response;
	type Error = E;
	type Future = Fut;

	fn call(&self, req: Request<RqB>) -> Self::Future {
		(self.func)(req)
	}
}

// ----------

impl<Func, A, B, Fut, R, E> Service<Request<B>> for HandlerFn<Func, (A, R)>
where
	Func: Fn(A) -> Fut + Clone + 'static,
	A: FromRequest<B>,
	B: 'static,
	Fut: Future<Output = Result<R, E>>,
	R: IntoResponse,
	E: Into<BoxedError>,
{
	type Response = Response;
	type Error = BoxedError;
	type Future = BoxedFuture<Result<Self::Response, Self::Error>>;

	#[inline]
	fn call(&self, req: Request<B>) -> Self::Future {
		let func_clone = self.func.clone();

		Box::pin(async move {
			let arg = match A::from_request(req) {
				Ok(v) => v,
				Err(Either::Left(v)) => return Ok(v.into_response()),
				Err(Either::Right(e)) => return Err(e.into()),
			};

			match (func_clone)(arg).await {
				Ok(v) => Ok(v.into_response()),
				Err(e) => Err(e.into()),
			}
		})
	}
}

impl<Func, A, B, Fut, R, E, S> IntoHandler<HandlerFn<Func, (A, R)>, B, S> for Func
where
	Func: Fn(A) -> Fut + Clone + Send + Sync + 'static,
	A: FromRequest<B> + 'static,
	B: Send + Sync + 'static,
	Fut: Future<Output = Result<R, E>>,
	R: IntoResponse + 'static,
	E: Into<BoxedError>,
	S: 'static,
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

impl<Func, A1, LA, B, Fut, R, E> Service<Request<B>> for HandlerFn<Func, (A1, LA, R)>
where
	Func: Fn(A1, LA) -> Fut + Clone + 'static,
	A1: FromRequestParts,
	LA: FromRequest<B>,
	B: 'static,
	Fut: Future<Output = Result<R, E>>,
	R: IntoResponse,
	E: Into<BoxedError>,
{
	type Response = Response;
	type Error = BoxedError;
	type Future = BoxedFuture<Result<Self::Response, Self::Error>>;

	#[inline]
	fn call(&self, req: Request<B>) -> Self::Future {
		let mut func_clone = self.func.clone(); // TODO: Maybe we should clone on a call side or we should clone the func?

		Box::pin(async move {
			let (parts, body) = req.into_parts();

			let arg1 = match A1::from_request_parts(&parts) {
				Ok(v) => v,
				Err(Either::Left(v)) => return Ok(v.into_response()),
				Err(Either::Right(e)) => return Err(e.into()),
			};

			let req = Request::<B>::from_parts(parts, body);

			let last_arg = match LA::from_request(req) {
				Ok(v) => v,
				Err(Either::Left(v)) => return Ok(v.into_response()),
				Err(Either::Right(e)) => return Err(e.into()),
			};

			match (func_clone)(arg1, last_arg).await {
				Ok(v) => Ok(v.into_response()),
				Err(e) => Err(e.into()),
			}
		})
	}
}

impl<Func, A1, LA, B, Fut, R, E, S> IntoHandler<HandlerFn<Func, (A1, LA, R)>, B, S> for Func
where
	Func: Fn(A1, LA) -> Fut + Clone + Send + Sync + 'static,
	A1: FromRequestParts + 'static,
	LA: FromRequest<B> + 'static,
	B: 'static,
	Fut: Future<Output = Result<R, E>>,
	R: IntoResponse + 'static,
	E: Into<BoxedError>,
	S: 'static,
{
	#[inline]
	fn into_handler(self) -> HandlerFn<Func, (A1, LA, R)> {
		HandlerFn {
			func: self,
			_mark: PhantomData,
		}
	}
}

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

#[cfg(test)]
mod test {
	use super::*;

	async fn handler_fn(req: Request) -> Result<Response, BoxedError> {
		todo!()
	}

	fn is_handler<H, B>(_: H)
	where
		H: Handler<B>,
		H::Response: IntoResponse,
		H::Error: Into<BoxedError>,
	{
	}

	fn is_into_handler<IH, H, B>(_: IH)
	where
		IH: IntoHandler<H, B>,
		H: Handler<B>,
		H::Response: IntoResponse,
		H::Error: Into<BoxedError>,
	{
	}

	fn test() {
		is_into_handler(handler_fn);

		let h = handler_fn.with_state(1u8).with_state(2u16);
		is_handler(h);

		let h = handler_fn.with_state(1u8);
		is_into_handler(h);

		// let mut hs = HandlerService(BoxedHandler::from(handler.into_handler()));
		// hs = hs.wrap_with(WrapperLayer{_mark: PhantomData});
		// is_handler(hs);

		// let hs = HandlerService::from_handler(handler);
		// is_handler(hs);
	}
}
