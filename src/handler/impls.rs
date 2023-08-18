use std::{future::Future, marker::PhantomData};

use crate::{
	request::{FromRequest, FromRequestParts},
	response::{IntoResponse, Response},
	utils::BoxedFuture,
};

use super::*;

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

macro_rules! impl_handler_fn {
	($(($($ps:ident),*),)? $($lp:ident)?) => {
		#[allow(non_snake_case)]
		impl<Func, M, $($($ps,)*)? $($lp,)? Fut, O, B> IntoHandler<(M, $($($ps,)*)? $($lp)?), B> for Func
		where
			Func: Fn($($($ps,)*)? $($lp)?) -> Fut,
			Fut: Future<Output = O>,
			O: IntoResponse,
			HandlerFn<Func, ($($($ps,)*)? $($lp)?)>: Handler<B>,
		{
			type Handler = HandlerFn<Func, ($($($ps,)*)? $($lp)?)>;

			fn into_handler(self) -> Self::Handler {
				HandlerFn::from(self)
			}
		}

		#[allow(non_snake_case)]
		impl<Func, M, $($($ps,)*)? $($lp,)? Fut, O, B> Handler<B> for HandlerFn<Func, (M, $($($ps,)*)? $($lp)?)>
		where
			Func: Fn($($($ps,)*)? $($lp)?) -> Fut + Clone + 'static,
			$($($ps: FromRequestParts,)*)?
			$($lp: FromRequest<B>,)?
			Fut: Future<Output = O>,
			O: IntoResponse,
			B: 'static,
		{
			type Response = Response;
			type Future = BoxedFuture<Response>;

			fn handle(&self, request: Request<B>) -> Self::Future {
				let func_clone = self.func.clone();

				Box::pin(async move {
					$(
						let (head, body) = request.into_parts();

						$(
							let $ps = match $ps::from_request_parts(&head).await {
								Ok(value) => value,
								Err(error) => return error.into_response(),
							};
						)*

						let request = Request::<B>::from_parts(head, body);
					)?

					$(
						let $lp = match $lp::from_request(request).await {
							Ok(value) => value,
							Err(error) => return error.into_response(),
						};
					)?

					func_clone($($($ps,)*)? $($lp)?).await.into_response()
				})
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
