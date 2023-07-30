use std::{future::Future, marker::PhantomData};

use crate::{
	body::IncomingBody,
	request::{FromRequest, FromRequestParts},
	response::{IntoResponse, Response},
	utils::{BoxedError, BoxedFuture},
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

impl<Func, Fut, E> Service<Request<IncomingBody>> for HandlerFn<Func, Request<IncomingBody>>
where
	Func: Fn(Request<IncomingBody>) -> Fut,
	Fut: Future<Output = Result<Response, E>>,
	E: Into<BoxedError>,
{
	type Response = Response;
	type Error = E;
	type Future = Fut;

	fn call(&self, req: Request<IncomingBody>) -> Self::Future {
		(self.func)(req)
	}
}

// ----------

impl<Func, A, Fut, R, E> Service<Request<IncomingBody>> for HandlerFn<Func, (A, R)>
where
	Func: Fn(A) -> Fut + Clone + 'static,
	A: FromRequest<IncomingBody>,
	IncomingBody: 'static,
	Fut: Future<Output = Result<R, E>>,
	R: IntoResponse,
	E: Into<BoxedError>,
{
	type Response = Response;
	type Error = BoxedError;
	type Future = BoxedFuture<Result<Self::Response, Self::Error>>;

	#[inline]
	fn call(&self, req: Request<IncomingBody>) -> Self::Future {
		let func_clone = self.func.clone();

		Box::pin(async move {
			let arg = match A::from_request(req).await {
				Ok(v) => v,
				Err(e) => return Err(e.into()),
			};

			match (func_clone)(arg).await {
				Ok(v) => Ok(v.into_response()),
				Err(e) => Err(e.into()),
			}
		})
	}
}

impl<Func, A, Fut, R, E, S> IntoHandler<HandlerFn<Func, (A, R)>, S> for Func
where
	Func: Fn(A) -> Fut + Clone + Send + Sync + 'static,
	A: FromRequest<IncomingBody> + 'static,
	IncomingBody: Send + 'static,
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

impl<Func, A1, LA, Fut, R, E> Service<Request<IncomingBody>> for HandlerFn<Func, (A1, LA, R)>
where
	Func: Fn(A1, LA) -> Fut + Clone + 'static,
	A1: FromRequestParts,
	LA: FromRequest<IncomingBody>,
	IncomingBody: 'static,
	Fut: Future<Output = Result<R, E>>,
	R: IntoResponse,
	E: Into<BoxedError>,
{
	type Response = Response;
	type Error = BoxedError;
	type Future = BoxedFuture<Result<Self::Response, Self::Error>>;

	#[inline]
	fn call(&self, req: Request<IncomingBody>) -> Self::Future {
		let mut func_clone = self.func.clone(); // TODO: Maybe we should clone on a call side or we should clone the func?

		Box::pin(async move {
			let (parts, body) = req.into_parts();

			let arg1 = match A1::from_request_parts(&parts).await {
				Ok(v) => v,
				Err(e) => return Err(e.into()),
			};

			let req = Request::<IncomingBody>::from_parts(parts, body);

			let last_arg = match LA::from_request(req).await {
				Ok(v) => v,
				Err(e) => return Err(e.into()),
			};

			match (func_clone)(arg1, last_arg).await {
				Ok(v) => Ok(v.into_response()),
				Err(e) => Err(e.into()),
			}
		})
	}
}

impl<Func, A1, LA, Fut, R, E, S> IntoHandler<HandlerFn<Func, (A1, LA, R)>, S> for Func
where
	Func: Fn(A1, LA) -> Fut + Clone + Send + Sync + 'static,
	A1: FromRequestParts + 'static,
	LA: FromRequest<IncomingBody> + 'static,
	IncomingBody: 'static,
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

	fn is_handler<H>(_: H)
	where
		H: Handler,
		H::Response: IntoResponse,
		H::Error: Into<BoxedError>,
	{
	}

	fn is_into_handler<IH, H>(_: IH)
	where
		IH: IntoHandler<H>,
		H: Handler,
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

		let boxed_handler = BoxedHandler::default();
		is_handler(boxed_handler);

		// let mut hs = HandlerService(BoxedHandler::from(handler.into_handler()));
		// hs = hs.wrap_with(WrapperLayer{_mark: PhantomData});
		// is_handler(hs);

		// let hs = HandlerService::from_handler(handler);
		// is_handler(hs);
	}
}
