use std::{
	any::Any,
	convert::Infallible,
	future::{ready, Future},
	marker::PhantomData,
	pin::{pin, Pin},
	task::{Context, Poll},
};

use argan_core::{
	body::{Body, HttpBody},
	BoxedError, BoxedFuture,
};
use bytes::Bytes;
use futures_util::FutureExt;
use pin_project::pin_project;

use crate::{
	common::{marker::Private, SCOPE_VALIDITY},
	data::extensions::NodeExtensions,
	request::{ExtractorGuard, FromRequest, Request, RequestContext, RequestHead, RequestHeadParts},
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
// Fn() -> Fut

impl<Func, Fut, O> IntoHandler<Private> for Func
where
	Func: Fn() -> Fut,
	Fut: Future<Output = O>,
	O: IntoResponseResult,
	HandlerFn<Func, Private>: Handler,
{
	type Handler = HandlerFn<Func, Private>;

	fn into_handler(self) -> Self::Handler {
		HandlerFn::from(self)
	}
}

impl<Ext, Func, Fut, O> Handler<Body, Ext> for HandlerFn<Func, Private>
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

pub struct HandlerFnFuture<Func>(Func);

impl<Func, Fut, O> Future for HandlerFnFuture<Func>
where
	Func: Fn() -> Fut + 'static,
	Fut: Future<Output = O>,
	O: IntoResponseResult,
{
	type Output = Result<Response, BoxedErrorResponse>;

	fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
		match pin!(self.0()).poll(cx) {
			Poll::Ready(value) => Poll::Ready(value.into_response_result()),
			Poll::Pending => Poll::Pending,
		}
	}
}

// // --------------------------------------------------
// // Fn(RequestHead) -> Fut
//
// impl<Func, Fut, O> IntoHandler<(Private, RequestHead)> for Func
// where
// 	Func: Fn(RequestHead) -> Fut,
// 	Fut: Future<Output = O>,
// 	O: IntoResponseResult,
// 	HandlerFn<Func, (Private, RequestHead)>: Handler,
// {
// 	type Handler = HandlerFn<Func, (Private, RequestHead)>;
//
// 	fn into_handler(self) -> Self::Handler {
// 		HandlerFn::from(self)
// 	}
// }
//
// impl<Ext, Func, Fut, O> Handler<Body, Ext> for HandlerFn<Func, (Private, RequestHead)>
// where
// 	Func: Fn(RequestHead) -> Fut + Clone + 'static,
// 	Fut: Future<Output = O>,
// 	O: IntoResponseResult,
// 	Ext: Clone + Sync + 'static,
// {
// 	type Response = Response;
// 	type Error = BoxedErrorResponse;
// 	type Future = HandlerFnRequestHeadFuture<Func>;
//
// 	#[inline(always)]
// 	fn handle(&self, request: RequestContext, _args: Args<'_, Ext>) -> Self::Future {
// 		let func_clone = self.func.clone();
// 		let (request, routing_state) = request.into_parts();
// 		let (head_parts, _) = request.into_parts();
//
// 		let head = RequestHead::new(head_parts, routing_state);
//
// 		HandlerFnRequestHeadFuture {
// 			func: func_clone,
// 			some_request_head: Some(head),
// 		}
// 	}
// }
//
// #[pin_project]
// pub struct HandlerFnRequestHeadFuture<Func> {
// 	func: Func,
// 	some_request_head: Option<RequestHead>,
// }
//
// impl<Func, Fut, O> Future for HandlerFnRequestHeadFuture<Func>
// where
// 	Func: Fn(RequestHead) -> Fut + 'static,
// 	Fut: Future<Output = O>,
// 	O: IntoResponseResult,
// {
// 	type Output = Result<Response, BoxedErrorResponse>;
//
// 	fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
// 		let self_projection = self.project();
//
// 		let request = self_projection
// 			.some_request_head
// 			.take()
// 			.expect("HandlerFnRequestFuture shouldn't be created without a request");
//
// 		match pin!((self_projection.func)(request)).poll(cx) {
// 			Poll::Ready(value) => Poll::Ready(value.into_response_result()),
// 			Poll::Pending => Poll::Pending,
// 		}
// 	}
// }
//
// // --------------------------------------------------
// // Fn(FromRequest) -> Fut
//
// impl<Func, T, Fut, O> IntoHandler<(Private, T)> for Func
// where
// 	Func: Fn(T) -> Fut,
// 	T: FromRequest,
// 	Fut: Future<Output = O>,
// 	O: IntoResponseResult,
// 	HandlerFn<Func, (Private, T)>: Handler,
// {
// 	type Handler = HandlerFn<Func, (Private, T)>;
//
// 	fn into_handler(self) -> Self::Handler {
// 		HandlerFn::from(self)
// 	}
// }
//
// impl<Ext, Func, T, Fut, O> Handler<Body, Ext> for HandlerFn<Func, (Private, T)>
// where
// 	Func: Fn(T) -> Fut + Clone + 'static,
// 	T: FromRequest,
// 	Fut: Future<Output = O>,
// 	O: IntoResponseResult,
// 	Ext: Clone + Sync + 'static,
// {
// 	type Response = Response;
// 	type Error = BoxedErrorResponse;
// 	type Future = HandlerFnExtractorFuture<Func, T, Fut, O>;
//
// 	#[inline(always)]
// 	fn handle(&self, request: RequestContext, _args: Args<'_, Ext>) -> Self::Future {
// 		let func_clone = self.func.clone();
// 		let (request, _) = request.into_parts();
//
// 		HandlerFnExtractorFuture {
// 			func: func_clone,
// 			some_request: Some(request),
// 			_output_mark: PhantomData,
// 		}
// 	}
// }
//
// #[pin_project]
// pub struct HandlerFnExtractorFuture<Func, T, Fut, O> {
// 	func: Func,
// 	some_request: Option<Request>,
// 	_output_mark: PhantomData<(fn(T) -> Fut, fn() -> O)>
// }
//
// impl<Func, T, Fut, O> Future for HandlerFnExtractorFuture<Func, T, Fut, O>
// where
// 	Func: Fn(T) -> Fut + 'static,
// 	T: FromRequest,
// 	Fut: Future<Output = O>,
// 	O: IntoResponseResult,
// {
// 	type Output = Result<Response, BoxedErrorResponse>;
//
// 	fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
// 		let self_projection = self.project();
//
// 		let request = self_projection
// 			.some_request
// 			.take()
// 			.expect("HandlerFnRequestFuture shouldn't be created without a request");
//
// 		let value = match pin!(T::from_request(request)).poll_unpin(cx) {
// 			Poll::Ready((head_parts, result)) => match result {
// 				Ok(value) => value,
// 				Err(error) => return Poll::Ready(Err(error.into())),
// 			},
// 			Poll::Pending => return Poll::Pending,
// 		};
//
// 		match pin!((self_projection.func)(value)).poll(cx) {
// 			Poll::Ready(value) => Poll::Ready(value.into_response_result()),
// 			Poll::Pending => Poll::Pending,
// 		}
// 	}
// }
//
// // --------------------------------------------------
//
// impl<Ext, Func, Fut, O> IntoHandler<(Private, Request, Args<'static, Ext>), Body, Ext> for Func
// where
// 	Ext: Clone,
// 	Func: Fn(RequestContext, Args<'static, Ext>) -> Fut + Clone,
// 	Fut: Future<Output = O>,
// 	O: IntoResponseResult,
// 	HandlerFn<Func, (Request, Args<'static, Ext>)>: Handler<Body, Ext>,
// {
// 	type Handler = HandlerFn<Func, (Request, Args<'static, Ext>)>;
//
// 	fn into_handler(self) -> Self::Handler {
// 		HandlerFn::from(self)
// 	}
// }
//
// impl<Ext, Func, Fut, O> Handler<Body, Ext> for HandlerFn<Func, (Request, Args<'static, Ext>)>
// where
// 	Ext: Clone,
// 	Func: Fn(RequestContext, Args<'static, Ext>) -> Fut + Clone,
// 	Fut: Future<Output = O>,
// 	O: IntoResponseResult,
// {
// 	type Response = Response;
// 	type Error = BoxedErrorResponse;
// 	type Future = HandlerFnRequestArgsFuture<Func, Ext>;
//
// 	#[inline(always)]
// 	fn handle(&self, request: RequestContext, mut args: Args<'_, Ext>) -> Self::Future {
// 		let func_clone = self.func.clone();
// 		let args = args.to_owned();
//
// 		HandlerFnRequestArgsFuture {
// 			func: func_clone,
// 			some_request: Some(request),
// 			some_args: Some(args),
// 		}
// 	}
// }
//
// #[pin_project]
// pub struct HandlerFnRequestArgsFuture<Func, Ext: Clone + 'static> {
// 	func: Func,
// 	some_request: Option<RequestContext>,
// 	some_args: Option<Args<'static, Ext>>,
// }
//
// impl<Ext, Func, Fut, O> Future for HandlerFnRequestArgsFuture<Func, Ext>
// where
// 	Ext: Clone + 'static,
// 	Func: Fn(RequestContext, Args<'static, Ext>) -> Fut,
// 	Fut: Future<Output = O>,
// 	O: IntoResponseResult,
// {
// 	type Output = Result<Response, BoxedErrorResponse>;
//
// 	fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
// 		let self_projection = self.project();
//
// 		let request = self_projection
// 			.some_request
// 			.take()
// 			.expect("HandlerFnRequestArgsFuture shouldn't be created without a request");
//
// 		let args = self_projection
// 			.some_args
// 			.take()
// 			.expect("HandlerFnFuture should be created with args");
//
// 		match pin!((self_projection.func)(request, args)).poll(cx) {
// 			Poll::Ready(value) => Poll::Ready(value.into_response_result()),
// 			Poll::Pending => Poll::Pending,
// 		}
// 	}
// }

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

// --------------------------------------------------
// Fn(RequestHead) -> Fut,
// Fn(T) -> Fut,
// Fn(RequestHead, T) -> Fut,

macro_rules! request_handler_fn {
	([$($RequestHead:ty)?] $($T:ident)?) => {
		impl<Func, $($T,)? Fut, O> IntoHandler<(Private, $($RequestHead,)? $($T)?), Body> for Func
		where
			Func: Fn($($RequestHead,)? $($T)?) -> Fut + Clone,
			$($T: FromRequest,)?
			Fut: Future<Output = O>,
			O: IntoResponseResult,
			HandlerFn<Func, (Private, $($RequestHead,)? $($T)?)>: Handler,
		{
			type Handler = HandlerFn<Func, (Private, $($RequestHead,)? $($T)?)>;

			fn into_handler(self) -> Self::Handler {
				HandlerFn::from(self)
			}
		}

		impl<Func, $($T,)? Fut, O> Handler for HandlerFn<Func, (Private, $($RequestHead,)? $($T)?)>
		where
			Func: Fn($($RequestHead,)? $($T)?) -> Fut + Clone,
			$($T: FromRequest,)?
			Fut: Future<Output = O>,
			O: IntoResponseResult,
		{
			type Response = Response;
			type Error = BoxedErrorResponse;
			type Future = HandlerFnRequestFuture<Func, (Fut, $($RequestHead,)? $($T,)? O)>;

			#[inline(always)]
			fn handle(&self, request: RequestContext, _args: Args) -> Self::Future {
				let func_clone = self.func.clone();

				HandlerFnRequestFuture {
					func: func_clone,
					some_request: Some(request),
					_mark: PhantomData,
				}
			}
		}

		#[allow(non_snake_case)]
		impl<Func, $($T,)? Fut, O>
			Future for HandlerFnRequestFuture<Func, (Fut, $($RequestHead,)? $($T,)? O)>
		where
			Func: Fn($($RequestHead,)? $($T)?) -> Fut,
			$($T: FromRequest,)?
			Fut: Future<Output = O>,
			O: IntoResponseResult,
		{
			type Output = Result<Response, BoxedErrorResponse>;

			fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
				let self_projection = self.project();

				let (request, routing_state, some_cookie_key) = self_projection
					.some_request
					.take()
					.expect("HandlerFnRequestFuture shouldn't be created without RequestContext")
					.into_parts();

				let (head_parts, body) = request.into_parts();

				$(
					let (head_parts, $T) = match pin!($T::from_request(head_parts, body))
						.poll_unpin(cx)
					{
						Poll::Ready((head_parts, result)) => match result {
							Ok(value) => (head_parts, value),
							Err(error) => return Poll::Ready(Err(error.into())),
						},
						Poll::Pending => return Poll::Pending,
					};
				)?

				$(
					let mut head = <$RequestHead>::new(head_parts, routing_state);

					if some_cookie_key.is_some() {
						head = head.with_cookie_key(some_cookie_key.expect(SCOPE_VALIDITY));
					}
				)?

				match pin!((self_projection.func)($(head as $RequestHead,)? $($T)?)).poll(cx) {
					Poll::Ready(value) => Poll::Ready(value.into_response_result()),
					Poll::Pending => Poll::Pending,
				}
			}
		}
	};
}

request_handler_fn!([RequestHead]);
request_handler_fn!([] T);
request_handler_fn!([RequestHead] T);

// ----------

#[pin_project]
pub struct HandlerFnRequestFuture<Func, Mark> {
	func: Func,
	some_request: Option<RequestContext>,
	_mark: PhantomData<(fn() -> (Mark))>,
}

// --------------------------------------------------

// impl<Func, T, Fut, O> IntoHandler<(Private, RequestHead, T), Body> for Func
// where
// 	Func: Fn(RequestHead, T) -> Fut + Clone,
// 	T: FromRequest,
// 	Fut: Future<Output = O>,
// 	O: IntoResponseResult,
// 	HandlerFn<Func, (RequestHead, T)>: Handler,
// {
// 	type Handler = HandlerFn<Func, (RequestHead, T)>;
//
// 	fn into_handler(self) -> Self::Handler {
// 		HandlerFn::from(self)
// 	}
// }
//
// impl<Func, T, Fut, O> Handler for HandlerFn<Func, (RequestHead, T)>
// where
// 	Func: Fn(RequestHead, T) -> Fut + Clone,
// 	T: FromRequest,
// 	Fut: Future<Output = O>,
// 	O: IntoResponseResult,
// {
// 	type Response = Response;
// 	type Error = BoxedErrorResponse;
// 	type Future = HandlerFnRequestFuture<Func, T, Fut, O>;
//
// 	#[inline(always)]
// 	fn handle(&self, request: RequestContext, _args: Args) -> Self::Future {
// 		let func_clone = self.func.clone();
//
// 		HandlerFnRequestFuture {
// 			func: func_clone,
// 			some_request: Some(request),
// 			_mark: PhantomData,
// 		}
// 	}
// }
//
// impl<Func, T, Fut, O> Future for HandlerFnRequestFuture<Func, T, Fut, O>
// where
// 	Func: Fn(RequestHead, T) -> Fut,
// 	T: FromRequest,
// 	Fut: Future<Output = O>,
// 	O: IntoResponseResult,
// {
// 	type Output = Result<Response, BoxedErrorResponse>;
//
// 	fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
// 		let self_projection = self.project();
//
// 		let (request, routing_state, some_cookie_key) = self_projection
// 			.some_request
// 			.take()
// 			.expect("HandlerFnRequestFuture shouldn't be created without RequestContext")
// 			.into_parts();
//
// 		let (head, value) = match pin!(T::from_request(request)).poll_unpin(cx) {
// 			Poll::Ready((head_parts, result)) => match result {
// 				Ok(value) => (RequestHead::new(head_parts, routing_state), value),
// 				Err(error) => return Poll::Ready(Err(error.into())),
// 			},
// 			Poll::Pending => return Poll::Pending,
// 		};
//
// 		if some_cookie_key.is_some() {
// 			head.with_cookie_key(some_cookie_key.expect(SCOPE_VALIDITY));
// 		}
//
// 		match pin!((self_projection.func)(head, value)).poll(cx) {
// 			Poll::Ready(value) => Poll::Ready(value.into_response_result()),
// 			Poll::Pending => Poll::Pending,
// 		}
// 	}
// }

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

// --------------------------------------------------
// Fn(G..., RequestHead) -> Fut
// Fn(G..., T) -> Fut
// Fn(G..., Args) -> Fut
// Fn(G..., RequestHead, T) -> Fut
// Fn(G..., RequestHead, Args) -> Fut
// Fn(G..., T, Args) -> Fut
// Fn(G..., RequestHead, T, Args) -> Fut

macro_rules! request_args_handler_fn {
	($($G:ident),* [$($RequestHead:ty)?] $($T:ident)? [$($Args:ty)?]) => {
		impl<Ext, Func, $($G,)* $($T,)? Fut, O>
			IntoHandler<(Private, $($G, RequestHead,)* $($RequestHead,)? $($T,)? $($Args)?), Body, Ext>
			for Func
		where
			Ext: Clone + 'static,
			Func: Fn($($G,)* $($RequestHead,)? $($T,)? $($Args)?) -> Fut + Clone,
			$($G: ExtractorGuard<Body, Ext>,)*
			$($T: FromRequest,)?
			Fut: Future<Output = O>,
			O: IntoResponseResult,
			HandlerFn<
				Func,
				(Private, $($G, RequestHead,)* $($RequestHead,)? $($T,)? $($Args)?)
			>: Handler<Body, Ext>,
		{
			type Handler = HandlerFn<
				Func,
				(Private, $($G, RequestHead,)* $($RequestHead,)? $($T,)? $($Args)?)
			>;

			fn into_handler(self) -> Self::Handler {
				HandlerFn::from(self)
			}
		}

		impl<Ext, Func, $($G,)* $($T,)? Fut, O> Handler<Body, Ext>
			for HandlerFn<Func, (Private, $($G, RequestHead,)* $($RequestHead,)? $($T,)? $($Args)?)>
		where
			Ext: Clone + 'static,
			Func: Fn($($G,)* $($RequestHead,)? $($T,)? $($Args)?) -> Fut + Clone,
			$($G: ExtractorGuard<Body, Ext>,)*
			$($T: FromRequest,)?
			Fut: Future<Output = O>,
			O: IntoResponseResult,
		{
			type Response = Response;
			type Error = BoxedErrorResponse;
			type Future = HandlerFnRequestArgsFuture<
				Func,
				Ext,
				($($G, RequestHead,)* $($RequestHead,)? $($T,)? $($Args,)? Fut, O)
			>;

			#[inline(always)]
			fn handle(&self, request: RequestContext, mut args: Args<'_, Ext>) -> Self::Future {
				let func_clone = self.func.clone();
				let args = args.to_owned();

				HandlerFnRequestArgsFuture {
					func: func_clone,
					some_request: Some(request),
					some_args: Some(args),
					_mark: PhantomData,
				}
			}
		}

		#[allow(non_snake_case)]
		impl<Func, $($G,)* $($T,)? Fut, O, Ext>
			Future
			for HandlerFnRequestArgsFuture<
				Func,
				Ext,
				($($G, RequestHead,)* $($RequestHead,)? $($T,)? $($Args,)? Fut, O)
			>
		where
			Func: Fn($($G,)* $($RequestHead,)? $($T,)? $($Args)?) -> Fut,
			$($G: ExtractorGuard<Body, Ext>,)*
			$($T: FromRequest,)?
			Fut: Future<Output = O>,
			O: IntoResponseResult,
			Ext: Clone + 'static,
		{
			type Output = Result<Response, BoxedErrorResponse>;

			fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
				let self_projection = self.project();

				let mut request = self_projection
					.some_request
					.take()
					.expect("HandlerFnRequestArgsFuture shouldn't be created without RequestContext");

				let args = self_projection
					.some_args
					.take()
					.expect("HandlerFnRequestArgsFuture shouldn't be created without Args");

				$(
					let $G = match pin!($G::from_request_context_and_args(&mut request, &args)).poll(cx) {
						Poll::Ready(result) => match result {
							Ok(g) => g,
							Err(error) => return Poll::Ready(Err(error.into())),
						},
						Poll::Pending => return Poll::Pending,
					};
				)*

				let (mut request, routing_state, some_cookie_key) = request.into_parts();

				let (head_parts, body) = request.into_parts();

				$(
					let (head_parts, $T) = match pin!($T::from_request(head_parts, body))
						.poll_unpin(cx)
					{
						Poll::Ready((head_parts, result)) => match result {
							Ok(value) => (head_parts, value),
							Err(error) => return Poll::Ready(Err(error.into())),
						},
						Poll::Pending => return Poll::Pending,
					};
				)?

				$(
					let mut head = <$RequestHead>::new(head_parts, routing_state);

					if some_cookie_key.is_some() {
						head = head.with_cookie_key(some_cookie_key.expect(SCOPE_VALIDITY));
					}
				)?

				match pin!((self_projection.func)(
					$($G,)*
					$(head as $RequestHead,)?
					$($T,)?
					$(args as $Args,)?
					)).poll(cx)
				{
					Poll::Ready(value) => Poll::Ready(value.into_response_result()),
					Poll::Pending => Poll::Pending,
				}
			}
		}
	};
}

request_args_handler_fn!([] [Args<'static, Ext>]);
request_args_handler_fn!([RequestHead] [Args<'static, Ext>]);
request_args_handler_fn!([] T [Args<'static, Ext>]);
request_args_handler_fn!([RequestHead] T [Args<'static, Ext>]);

macro_rules! call_for_tuples {
	([$($RH:ty)?] $($T:ident)? [$($Args:ty)?]) => {
		request_args_handler_fn!(G1 [$($RH)?] $($T)? [$($Args)?]);
		request_args_handler_fn!(G1, G2 [$($RH)?] $($T)? [$($Args)?]);
		request_args_handler_fn!(G1, G2, G3 [$($RH)?] $($T)? [$($Args)?]);
		request_args_handler_fn!(G1, G2, G3, G4 [$($RH)?] $($T)? [$($Args)?]);
		request_args_handler_fn!(G1, G2, G3, G4, G5 [$($RH)?] $($T)? [$($Args)?]);
		request_args_handler_fn!(G1, G2, G3, G4, G5, G6 [$($RH)?] $($T)? [$($Args)?]);
		request_args_handler_fn!(G1, G2, G3, G4, G5, G6, G7 [$($RH)?] $($T)? [$($Args)?]);
		request_args_handler_fn!(G1, G2, G3, G4, G5, G6, G7, G8 [$($RH)?] $($T)? [$($Args)?]);
		request_args_handler_fn!(G1, G2, G3, G4, G5, G6, G7, G8, G9 [$($RH)?] $($T)? [$($Args)?]);
		request_args_handler_fn!(G1, G2, G3, G4, G5, G6, G7, G8, G9, G10 [$($RH)?] $($T)? [$($Args)?]);
		request_args_handler_fn!(
			G1, G2, G3, G4, G5, G6, G7, G8, G9, G10, G11
			[$($RH)?] $($T)? [$($Args)?]
		);
		request_args_handler_fn!(
			G1, G2, G3, G4, G5, G6, G7, G8, G9, G10, G11, G12
			[$($RH)?] $($T)? [$($Args)?]
		);
	};
}

call_for_tuples!([] []);
call_for_tuples!([RequestHead] []);
call_for_tuples!([RequestHead] [Args<'static, Ext>]);
call_for_tuples!([] T []);
call_for_tuples!([] T [Args<'static, Ext>]);
call_for_tuples!([] [Args<'static, Ext>]);
call_for_tuples!([RequestHead] T []);
call_for_tuples!([RequestHead] T [Args<'static, Ext>]);

// ----------

#[pin_project]
pub struct HandlerFnRequestArgsFuture<Func, Ext, Mark>
where
	Ext: Clone + 'static,
{
	func: Func,
	some_request: Option<RequestContext>,
	some_args: Option<Args<'static, Ext>>,
	_mark: PhantomData<(fn() -> (Mark))>,
}

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

pub fn is_handler<H, Mark>(handler: H)
where
	H: IntoHandler<Mark, Body>,
	H::Handler: Handler + Clone + Send + Sync + 'static,
	<H::Handler as Handler>::Response: IntoResponse,
	<H::Handler as Handler>::Error: Into<BoxedErrorResponse>,
	<H::Handler as Handler>::Future: Send,
{
}

fn test() {
	is_handler(|| async {});
	is_handler(|_: RequestHead| async {});
	is_handler(|_: AB| async {});
	is_handler(|_: RequestHead, _: AB| async {});
	is_handler(|_: Args<'static>| async {});
	is_handler(|_: RequestHead, _: Args<'static>| async {});
	is_handler(|_: AB, _: Args<'static>| async {});
	is_handler(|_: RequestHead, _: AB, _: Args<'static>| async {});

	is_handler(|_: BB| async {});
	is_handler(|_: BB, _: RequestHead| async {});
	is_handler(|_: BB, _: RequestHead, _: Args<'static>| async {});
	is_handler(|_: BB, _: AB| async {});
	is_handler(|_: BB, _: AB, _: Args<'static>| async {});
	is_handler(|_: BB, _: Args<'static>| async {});
	is_handler(|_: BB, _: RequestHead, _: AB| async {});
	is_handler(|_: BB, _: RequestHead, _: AB, _: Args<'static>| async {});

	is_handler(|_: BB, _: BB| async {});
	is_handler(|_: BB, _: BB, _: RequestHead| async {});
	is_handler(|_: BB, _: BB, _: RequestHead, _: Args<'static>| async {});
	is_handler(|_: BB, _: BB, _: AB| async {});
	is_handler(|_: BB, _: BB, _: AB, _: Args<'static>| async {});
	is_handler(|_: BB, _: BB, _: Args<'static>| async {});
	is_handler(|_: BB, _: BB, _: RequestHead, _: AB| async {});
	is_handler(|_: BB, _: BB, _: RequestHead, _: AB, _: Args<'static>| async {});

	is_handler(|_: BB, _: BB, _: BB| async {});
	is_handler(|_: BB, _: BB, _: BB, _: RequestHead| async {});
	is_handler(|_: BB, _: BB, _: BB, _: RequestHead, _: Args<'static>| async {});
	is_handler(|_: BB, _: BB, _: BB, _: AB| async {});
	is_handler(|_: BB, _: BB, _: BB, _: AB, _: Args<'static>| async {});
	is_handler(|_: BB, _: BB, _: BB, _: Args<'static>| async {});
	is_handler(|_: BB, _: BB, _: BB, _: RequestHead, _: AB| async {});
	is_handler(|_: BB, _: BB, _: BB, _: RequestHead, _: AB, _: Args<'static>| async {});
}

struct AB;

impl<B> FromRequest<B> for AB {
	type Error = Infallible;

	fn from_request(
		head: RequestHeadParts,
		body: B,
	) -> impl Future<Output = (RequestHeadParts, Result<Self, Self::Error>)> + Send {
		ready((head, Ok(AB)))
	}
}

struct BB;

impl<B, Ext: Clone> ExtractorGuard<B, Ext> for BB {
	type Error = Infallible;

	fn from_request_context_and_args(
		request: &mut RequestContext<B>,
		args: &Args<'static, Ext>,
	) -> impl Future<Output = Result<Self, Self::Error>> + Send {
		ready(Ok(BB))
	}
}

// impl<Ext, Func, G, T, Fut, O> IntoHandler<(Private, G, RequestHead, T, Args<'static, Ext>), Body, Ext>
// 	for Func
// where
// 	Ext: Clone,
// 	Func: Fn(G, RequestHead, T, Args<'static, Ext>) -> Fut + Clone,
// 	G: HandlerExtractorGuard<Body, Ext>,
// 	T: FromRequest,
// 	Fut: Future<Output = O>,
// 	O: IntoResponseResult,
// 	HandlerFn<Func, (Private, G, RequestHead, T, Args<'static, Ext>)>: Handler<Body, Ext>,
// {
// 	type Handler = HandlerFn<Func, (Private, G, RequestHead, T, Args<'static, Ext>)>;
//
// 	fn into_handler(self) -> Self::Handler {
// 		HandlerFn::from(self)
// 	}
// }
//
// impl<Ext, Func, G, T, Fut, O> Handler<Body, Ext> for HandlerFn<Func, (Private, G, RequestHead, T, Args<'static, Ext>)>
// where
// 	Ext: Clone,
// 	Func: Fn(G, RequestHead, T, Args<'static, Ext>) -> Fut + Clone,
// 	G: HandlerExtractorGuard<Body, Ext>,
// 	T: FromRequest,
// 	Fut: Future<Output = O>,
// 	O: IntoResponseResult,
// {
// 	type Response = Response;
// 	type Error = BoxedErrorResponse;
// 	type Future = HandlerFnRequestArgsFuture<Func, Ext, (G, T, Fut, O)>;
//
// 	#[inline(always)]
// 	fn handle(&self, request: RequestContext, mut args: Args<'_, Ext>) -> Self::Future {
// 		let func_clone = self.func.clone();
// 		let args = args.to_owned();
//
// 		HandlerFnRequestArgsFuture {
// 			func: func_clone,
// 			some_request: Some(request),
// 			some_args: Some(args),
// 			_mark: PhantomData,
// 		}
// 	}
// }
//
// impl<Func, G, T, Fut, O, Ext> Future for HandlerFnRequestArgsFuture<Func, Ext, (G, T, Fut, O)>
// where
// 	Func: Fn(G, RequestHead, T, Args<'static, Ext>) -> Fut,
// 	G: HandlerExtractorGuard<Body, Ext>,
// 	T: FromRequest,
// 	Fut: Future<Output = O>,
// 	O: IntoResponseResult,
// 	Ext: Clone + 'static,
// {
// 	type Output = Result<Response, BoxedErrorResponse>;
//
// 	fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
// 		let self_projection = self.project();
//
// 		let request = self_projection
// 			.some_request
// 			.take()
// 			.expect("HandlerFnRequestArgsFuture shouldn't be created without RequestContext");
//
// 		let args = self_projection
// 			.some_args
// 			.take()
// 			.expect("HandlerFnRequestArgsFuture shouldn't be created without Args");
//
// 		let g = match pin!(G::from_request_head_and_args(&mut request, &args)).poll(cx) {
// 			Poll::Ready(result) => match result {
// 				Ok(g) => g,
// 				Err(error) => return Poll::Ready(Err(error.into())),
// 			},
// 			Poll::Pending => return Poll::Pending,
// 		};
//
// 		let (request, routing_state, some_cookie_key) = request.into_parts();
//
// 		let mut some_head_parts = Option::<RequestHeadParts>::None;
//
// 		let (some_head_parts, value) = match pin!(T::from_request(request)).poll_unpin(cx) {
// 			Poll::Ready((head_parts, result)) => match result {
// 				Ok(value) => (Some(head_parts), value),
// 				Err(error) => return Poll::Ready(Err(error.into())),
// 			},
// 			Poll::Pending => return Poll::Pending,
// 		};
//
// 		let head = if let Some(head_parts) = some_head_parts {
// 			RequestHead::new(head_parts, routing_state)
// 		} else {
// 			let (head_parts, _) = request.into_parts();
//
// 			RequestHead::new(head_parts, routing_state)
// 		};
//
// 		if some_cookie_key.is_some() {
// 			head.with_cookie_key(some_cookie_key.expect(SCOPE_VALIDITY));
// 		}
//
// 		match pin!((self_projection.func)(g, head, value, args)).poll(cx) {
// 			Poll::Ready(value) => Poll::Ready(value.into_response_result()),
// 			Poll::Pending => Poll::Pending,
// 		}
// 	}
// }

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

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
