use std::{
	any::Any,
	future::Future,
	marker::PhantomData,
	pin::{pin, Pin},
	task::{Context, Poll},
};

use argan_core::{
	body::{Body, HttpBody},
	BoxedError, BoxedFuture,
};
use bytes::Bytes;
use pin_project::pin_project;

use crate::{
	common::marker::Private,
	data::extensions::NodeExtensions,
	request::{FromRequest, Request, RequestContext},
	response::{BoxedErrorResponse, IntoResponse, IntoResponseResult, Response},
	routing::RoutingState,
};

use super::{Args, BoxedHandler, Handler, IntoHandler};

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

// --------------------------------------------------
// AdaptiveHandler

#[derive(Clone)]
pub struct AdaptiveHandler(BoxedHandler);

impl<B> Handler<B> for AdaptiveHandler
where
	B: HttpBody<Data = Bytes> + Send + Sync + 'static,
	B::Error: Into<BoxedError>,
{
	type Response = Response;
	type Error = BoxedErrorResponse;
	type Future = BoxedFuture<Result<Self::Response, Self::Error>>;

	#[inline(always)]
	fn handle(&self, request: RequestContext<B>, args: Args<'_, ()>) -> Self::Future {
		self.0.handle(request.map(Body::new), args)
	}
}

impl From<BoxedHandler> for AdaptiveHandler {
	#[inline(always)]
	fn from(boxed_handler: BoxedHandler) -> Self {
		Self(boxed_handler)
	}
}

// --------------------------------------------------
// HandlerFn

#[derive(Debug)]
pub struct HandlerFn<Func, Mark> {
	func: Func,
	_mark: PhantomData<fn() -> Mark>,
}

impl<Func, Mark> From<Func> for HandlerFn<Func, Mark> {
	fn from(func: Func) -> Self {
		Self {
			func,
			_mark: PhantomData,
		}
	}
}

impl<Func, Mark> Clone for HandlerFn<Func, Mark>
where
	Func: Clone,
{
	fn clone(&self) -> Self {
		Self {
			func: self.func.clone(),
			_mark: PhantomData,
		}
	}
}

// --------------------------------------------------

impl<Func, Fut, O> IntoHandler<(Private)> for Func
where
	Func: Fn() -> Fut,
	Fut: Future<Output = O>,
	O: IntoResponseResult,
	HandlerFn<Func, ()>: Handler,
{
	type Handler = HandlerFn<Func, ()>;

	fn into_handler(self) -> Self::Handler {
		HandlerFn::from(self)
	}
}

impl<Ext, Func, Fut, O> Handler<Body, Ext> for HandlerFn<Func, ()>
where
	Ext: Clone,
	Func: Fn() -> Fut + Clone + 'static,
	Fut: Future<Output = O>,
	O: IntoResponseResult,
{
	type Response = Response;
	type Error = BoxedErrorResponse;
	type Future = HandlerFnFuture<Func>;

	#[inline(always)]
	fn handle(&self, _: RequestContext, _: Args<'_, Ext>) -> Self::Future {
		let func_clone = self.func.clone();

		HandlerFnFuture(func_clone)
	}
}

#[pin_project]
pub struct HandlerFnFuture<Func>(Func);

impl<Func, Fut, O> Future for HandlerFnFuture<Func>
where
	Func: Fn() -> Fut + 'static,
	Fut: Future<Output = O>,
	O: IntoResponseResult,
{
	type Output = Result<Response, BoxedErrorResponse>;

	fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
		let self_projection = self.project();

		match pin!((self_projection.0)()).poll(cx) {
			Poll::Ready(value) => Poll::Ready(value.into_response_result()),
			Poll::Pending => Poll::Pending,
		}
	}
}

// --------------------------------------------------

impl<Func, Fut, O> IntoHandler<(Private, Request)> for Func
where
	Func: Fn(RequestContext) -> Fut,
	Fut: Future<Output = O>,
	O: IntoResponseResult,
	HandlerFn<Func, Request>: Handler,
{
	type Handler = HandlerFn<Func, Request>;

	fn into_handler(self) -> Self::Handler {
		HandlerFn::from(self)
	}
}

impl<Ext, Func, Fut, O> Handler<Body, Ext> for HandlerFn<Func, Request>
where
	Func: Fn(RequestContext) -> Fut + Clone + 'static,
	Fut: Future<Output = O>,
	O: IntoResponseResult,
	Ext: Clone + Sync + 'static,
{
	type Response = Response;
	type Error = BoxedErrorResponse;
	type Future = HandlerFnRequestFuture<Func>;

	#[inline(always)]
	fn handle(&self, request: RequestContext, _args: Args<'_, Ext>) -> Self::Future {
		let func_clone = self.func.clone();

		HandlerFnRequestFuture {
			func: func_clone,
			some_request: Some(request),
		}
	}
}

#[pin_project]
pub struct HandlerFnRequestFuture<Func> {
	func: Func,
	some_request: Option<RequestContext>,
}

impl<Func, Fut, O> Future for HandlerFnRequestFuture<Func>
where
	Func: Fn(RequestContext) -> Fut + 'static,
	Fut: Future<Output = O>,
	O: IntoResponseResult,
{
	type Output = Result<Response, BoxedErrorResponse>;

	fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
		let self_projection = self.project();

		let request = self_projection
			.some_request
			.take()
			.expect("HandlerFnRequestFuture shouldn't be created without a request");

		match pin!((self_projection.func)(request)).poll(cx) {
			Poll::Ready(value) => Poll::Ready(value.into_response_result()),
			Poll::Pending => Poll::Pending,
		}
	}
}

// --------------------------------------------------

impl<Ext, Func, Fut, O> IntoHandler<(Private, Request, Args<'static, Ext>), Body, Ext> for Func
where
	Ext: Clone,
	Func: Fn(RequestContext, Args<'static, Ext>) -> Fut + Clone,
	Fut: Future<Output = O>,
	O: IntoResponseResult,
	HandlerFn<Func, (Request, Args<'static, Ext>)>: Handler<Body, Ext>,
{
	type Handler = HandlerFn<Func, (Request, Args<'static, Ext>)>;

	fn into_handler(self) -> Self::Handler {
		HandlerFn::from(self)
	}
}

impl<Ext, Func, Fut, O> Handler<Body, Ext> for HandlerFn<Func, (Request, Args<'static, Ext>)>
where
	Ext: Clone,
	Func: Fn(RequestContext, Args<'static, Ext>) -> Fut + Clone,
	Fut: Future<Output = O>,
	O: IntoResponseResult,
{
	type Response = Response;
	type Error = BoxedErrorResponse;
	type Future = HandlerFnRequestArgsFuture<Func, Ext>;

	#[inline(always)]
	fn handle(&self, request: RequestContext, mut args: Args<'_, Ext>) -> Self::Future {
		let func_clone = self.func.clone();
		let args = args.to_owned();

		HandlerFnRequestArgsFuture {
			func: func_clone,
			some_request: Some(request),
			some_args: Some(args),
		}
	}
}

#[pin_project]
pub struct HandlerFnRequestArgsFuture<Func, Ext: Clone + 'static> {
	func: Func,
	some_request: Option<RequestContext>,
	some_args: Option<Args<'static, Ext>>,
}

impl<Ext, Func, Fut, O> Future for HandlerFnRequestArgsFuture<Func, Ext>
where
	Ext: Clone + 'static,
	Func: Fn(RequestContext, Args<'static, Ext>) -> Fut,
	Fut: Future<Output = O>,
	O: IntoResponseResult,
{
	type Output = Result<Response, BoxedErrorResponse>;

	fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
		let self_projection = self.project();

		let request = self_projection
			.some_request
			.take()
			.expect("HandlerFnRequestArgsFuture shouldn't be created without a request");

		let args = self_projection
			.some_args
			.take()
			.expect("HandlerFnFuture should be created with args");

		match pin!((self_projection.func)(request, args)).poll(cx) {
			Poll::Ready(value) => Poll::Ready(value.into_response_result()),
			Poll::Pending => Poll::Pending,
		}
	}
}

// --------------------------------------------------

// #[rustfmt::skip]
// macro_rules! impl_handler_fn {
// 	($(($($ps:ident),*),)? $($lp:ident)?) => {
// 		#[allow(non_snake_case)]
// 		impl<Func, $($($ps,)*)? $($lp,)? Fut, O, B, Ext>
// 		IntoHandler<(Private, $($($ps,)*)? $($lp)?), B, Ext>
// 		for Func
// 		where
// 			Func: Fn($($($ps,)*)? $($lp)?) -> Fut,
// 			Fut: Future<Output = O>,
// 			O: IntoResponseResult,
// 			Ext: Clone,
// 			HandlerFn<Func, (Private, $($($ps,)*)? $($lp)?)>: Handler<B, Ext>,
// 		{
// 			type Handler = HandlerFn<Func, (Private, $($($ps,)*)? $($lp)?)>;
//
// 			fn into_handler(self) -> Self::Handler {
// 				HandlerFn::from(self)
// 			}
// 		}
//
// 		#[allow(non_snake_case)]
// 		impl<Func, $($($ps,)*)? $($lp,)? Fut, O, B, Ext> Handler<B, Ext>
// 		for HandlerFn<Func, (Private, $($($ps,)*)? $($lp)?)>
// 		where
// 			Func: Fn($($($ps,)*)? $($lp)?) -> Fut + Clone + 'static,
// 			$($($ps: for <'n> FromRequestHead<Args<'n, Ext>>,)*)?
// 			$($lp: for <'n> FromRequest<B, Args<'n, Ext>>,)?
// 			Fut: Future<Output = O>,
// 			O: IntoResponseResult,
// 			B: 'static,
// 			Ext: Clone + Sync + 'static,
// 		{
// 			type Response = Response;
// 			type Error = BoxedErrorResponse;
// 			type Future = HandlerFnFuture<Func, (Private, $($($ps,)*)? $($lp)?), B, Ext>;
//
// 			fn handle(&self, request: Request<B>, mut args: Args<'_, Ext>) -> Self::Future {
// 				let func_clone = self.func.clone();
// 				let args = args.to_owned();
//
// 				HandlerFnFuture::new(func_clone, request, args)
// 			}
// 		}
//
// 		#[allow(non_snake_case)]
// 		impl<Func, $($($ps,)*)? $($lp,)? Fut, O, B, Ext: Clone> Future
// 		for HandlerFnFuture<Func, (Private, $($($ps,)*)? $($lp)?), B, Ext>
// 		where
// 			Func: Fn($($($ps,)*)? $($lp)?) -> Fut + Clone + 'static,
// 			$($($ps: for <'n> FromRequestHead<Args<'n, Ext>>,)*)?
// 			$($lp: for <'n> FromRequest<B, Args<'n, Ext>>,)?
// 			Fut: Future<Output = O>,
// 			O: IntoResponseResult,
// 			B: 'static,
// 			Ext: Sync,
// 		{
// 			type Output = Result<Response, BoxedErrorResponse>;
//
// 			fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
// 				let self_projection = self.project();
//
// 				let mut args = self_projection.some_args.take().expect(
// 					"HandlerFnFuture should be created with args",
// 				);
//
// 				$(
// 					let (mut head, body) = self_projection.some_request.take().expect(
// 						"HandlerFnFuture should be created with a request",
// 					).into_parts();
//
// 					$(
// 						let $ps = match pin!($ps::from_request_head(&mut head, &args)).poll(cx) {
// 							Poll::Ready(result) => {
// 								match result {
// 									Ok(value) => value,
// 									Err(error) => return Poll::Ready(Err(error.into())),
// 								}
// 							},
// 							Poll::Pending => return Poll::Pending,
// 						};
// 					)*
//
// 					self_projection.some_request.replace(Request::<B>::from_parts(head, body));
// 				)?
//
// 				$(
// 					let $lp =
// 						match pin!(
// 							$lp::from_request(
// 								self_projection.some_request.take().expect(
// 									"the constructor of the HandlerFnFuture or the local scope should set the request"
// 								),
// 								args,
// 							)
// 						).poll(cx) {
// 							Poll::Ready(result) => {
// 								match result {
// 									Ok(value) => value,
// 									Err(error) => return Poll::Ready(Err(error.into())),
// 								}
// 							}
// 							Poll::Pending => return Poll::Pending,
// 						};
// 				)?
//
// 				match pin!((self_projection.func)($($($ps,)*)? $($lp)?)).poll(cx) {
// 					Poll::Ready(value) => Poll::Ready(value.into_response_result()),
// 					Poll::Pending => Poll::Pending,
// 				}
// 			}
// 		}
// 	};
// }
//
// impl_handler_fn!();
// impl_handler_fn!(LP);
// impl_handler_fn!((P1), LP);
// impl_handler_fn!((P1, P2), LP);
// impl_handler_fn!((P1, P2, P3), LP);
// impl_handler_fn!((P1, P2, P3, P4), LP);
// impl_handler_fn!((P1, P2, P3, P4, P5), LP);
// impl_handler_fn!((P1, P2, P3, P4, P5, P6), LP);
// impl_handler_fn!((P1, P2, P3, P4, P5, P6, P7), LP);
// impl_handler_fn!((P1, P2, P3, P4, P5, P6, P7, P8), LP);
// impl_handler_fn!((P1, P2, P3, P4, P5, P6, P7, P8, P9), LP);
// impl_handler_fn!((P1, P2, P3, P4, P5, P6, P7, P8, P9, P10), LP);
// impl_handler_fn!((P1, P2, P3, P4, P5, P6, P7, P8, P9, P10, P11), LP);
// impl_handler_fn!((P1, P2, P3, P4, P5, P6, P7, P8, P9, P10, P11, P12), LP);
// impl_handler_fn!((P1, P2, P3, P4, P5, P6, P7, P8, P9, P10, P11, P12, P13), LP);
// #[rustfmt::skip]
// impl_handler_fn!((P1, P2, P3, P4, P5, P6, P7, P8, P9, P10, P11, P12, P13, P14), LP);
// #[rustfmt::skip]
// impl_handler_fn!((P1, P2, P3, P4, P5, P6, P7, P8, P9, P10, P11, P12, P13, P14, P15), LP);
//
// // --------------------------------------------------
//
// #[pin_project]
// pub struct HandlerFnFuture<Func, Mark, B, Ext: Clone + 'static> {
// 	func: Func,
// 	some_request: Option<Request<B>>,
// 	some_args: Option<Args<'static, Ext>>,
// 	_mark: PhantomData<fn() -> Mark>,
// }
//
// impl<Func, Mark, B, Ext: Clone> HandlerFnFuture<Func, Mark, B, Ext> {
// 	fn new(func: Func, request: Request<B>, args: Args<'static, Ext>) -> Self {
// 		Self {
// 			func,
// 			some_request: Some(request),
// 			some_args: Some(args),
// 			_mark: PhantomData,
// 		}
// 	}
// }
