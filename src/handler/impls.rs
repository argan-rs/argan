use std::{
	future::Future,
	marker::PhantomData,
	pin::{pin, Pin},
	task::{Context, Poll},
};

use pin_project::pin_project;

use crate::{
	request::{FromRequest, FromRequestHead, Request},
	response::{IntoResponse, Response},
	utils::{mark::Private, BoxedFuture},
};

use super::{Handler, IntoHandler};

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct HandlerFn<Func, M> {
	func: Func,
	_mark: PhantomData<fn() -> M>,
}

impl<Func, M> From<Func> for HandlerFn<Func, M> {
	fn from(func: Func) -> Self {
		Self {
			func,
			_mark: PhantomData,
		}
	}
}

// --------------------------------------------------

impl<Func> IntoHandler<Request> for Func
where
	Func: Fn(Request) -> BoxedFuture<Response>,
	HandlerFn<Func, Request>: Handler,
{
	type Handler = HandlerFn<Func, Request>;

	fn into_handler(self) -> Self::Handler {
		HandlerFn::from(self)
	}
}

impl<Func> Handler for HandlerFn<Func, Request>
where
	Func: Fn(Request) -> BoxedFuture<Response>,
{
	type Response = Response;
	type Future = BoxedFuture<Self::Response>;

	fn handle(&self, request: Request) -> Self::Future {
		(self.func)(request)
	}
}

// --------------------------------------------------

macro_rules! impl_handler_fn {
	($(($($ps:ident),*),)? $($lp:ident)?) => {
		#[allow(non_snake_case)]
		impl<Func, $($($ps,)*)? $($lp,)? Fut, O, B> IntoHandler<(Private, $($($ps,)*)? $($lp)?), B> for Func
		where
			Func: Fn($($($ps,)*)? $($lp)?) -> Fut,
			Fut: Future<Output = O>,
			O: IntoResponse,
			HandlerFn<Func, (Private, $($($ps,)*)? $($lp)?)>: Handler<B>,
		{
			type Handler = HandlerFn<Func, (Private, $($($ps,)*)? $($lp)?)>;

			fn into_handler(self) -> Self::Handler {
				HandlerFn::from(self)
			}
		}

		#[allow(non_snake_case)]
		impl<Func, $($($ps,)*)? $($lp,)? Fut, O, B> Handler<B> for HandlerFn<Func, (Private, $($($ps,)*)? $($lp)?)>
		where
			Func: Fn($($($ps,)*)? $($lp)?) -> Fut + Clone + 'static,
			$($($ps: FromRequestHead,)*)?
			$($lp: FromRequest<B>,)?
			Fut: Future<Output = O>,
			O: IntoResponse,
			B: 'static,
		{
			type Response = Response;
			type Future = HandlerFnFuture<Func, (Private, $($($ps,)*)? $($lp)?), B>;

			fn handle(&self, request: Request<B>) -> Self::Future {
				let func_clone = self.func.clone();

				HandlerFnFuture::new(func_clone, request)
			}
		}

		#[allow(non_snake_case)]
		impl<Func, $($($ps,)*)? $($lp,)? Fut, O, B> Future for HandlerFnFuture<Func, (Private, $($($ps,)*)? $($lp)?), B>
		where
			Func: Fn($($($ps,)*)? $($lp)?) -> Fut + Clone + 'static,
			$($($ps: FromRequestHead,)*)?
			$($lp: FromRequest<B>,)?
			Fut: Future<Output = O>,
			O: IntoResponse,
			B: 'static,
		{
			type Output = Response;

			fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
				let self_projection = self.project();

				$(
					let (mut head, body) = self_projection.some_request.take().unwrap().into_parts();

					$(
						let $ps = match pin!($ps::from_request_head(&mut head)).poll(cx) {
							Poll::Ready(result) => {
								match result {
									Ok(value) => value,
									Err(error) => return Poll::Ready(error.into_response()),
								}
							},
							Poll::Pending => return Poll::Pending,
						};
					)*

					self_projection.some_request.replace(Request::<B>::from_parts(head, body));
				)?

				$(
					let $lp = match pin!($lp::from_request(self_projection.some_request.take().unwrap())).poll(cx) {
						Poll::Ready(result) => {
							match result {
								Ok(value) => value,
								Err(error) => return Poll::Ready(error.into_response()),
							}
						}
						Poll::Pending => return Poll::Pending,
					};
				)?

				match pin!((self_projection.func)($($($ps,)*)? $($lp)?)).poll(cx) {
					Poll::Ready(value) => Poll::Ready(value.into_response()),
					Poll::Pending => Poll::Pending,
				}
			}
		}
	};
}

impl_handler_fn!();
impl_handler_fn!(LP);
impl_handler_fn!((P1), LP);
impl_handler_fn!((P1, P2), LP);
impl_handler_fn!((P1, P2, P3), LP);
impl_handler_fn!((P1, P2, P3, P4), LP);
impl_handler_fn!((P1, P2, P3, P4, P5), LP);
impl_handler_fn!((P1, P2, P3, P4, P5, P6), LP);
impl_handler_fn!((P1, P2, P3, P4, P5, P6, P7), LP);
impl_handler_fn!((P1, P2, P3, P4, P5, P6, P7, P8), LP);
impl_handler_fn!((P1, P2, P3, P4, P5, P6, P7, P8, P9), LP);
impl_handler_fn!((P1, P2, P3, P4, P5, P6, P7, P8, P9, P10), LP);
impl_handler_fn!((P1, P2, P3, P4, P5, P6, P7, P8, P9, P10, P11), LP);
impl_handler_fn!((P1, P2, P3, P4, P5, P6, P7, P8, P9, P10, P11, P12), LP);
impl_handler_fn!((P1, P2, P3, P4, P5, P6, P7, P8, P9, P10, P11, P12, P13), LP);
impl_handler_fn!(
	(P1, P2, P3, P4, P5, P6, P7, P8, P9, P10, P11, P12, P13, P14),
	LP
);
impl_handler_fn!(
	(P1, P2, P3, P4, P5, P6, P7, P8, P9, P10, P11, P12, P13, P14, P15),
	LP
);

// --------------------------------------------------

#[pin_project]
pub struct HandlerFnFuture<Func, M, B> {
	func: Func,
	some_request: Option<Request<B>>,
	_mark: PhantomData<fn() -> M>,
}

impl<Func, M, B> HandlerFnFuture<Func, M, B> {
	fn new(func: Func, request: Request<B>) -> Self {
		Self {
			func,
			some_request: Some(request),
			_mark: PhantomData,
		}
	}
}
