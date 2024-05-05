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
	fn handle(&self, request_context: RequestContext<B>, args: Args<'_, ()>) -> Self::Future {
		self.0.handle(request_context.map(Body::new), args)
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
	Fut: Future<Output = O> + Send + 'static,
	O: IntoResponseResult,
{
	type Response = Response;
	type Error = BoxedErrorResponse;
	type Future = BoxedFuture<Result<Self::Response, Self::Error>>;

	#[inline(always)]
	fn handle(&self, _: RequestContext, _: Args<'_, Ext>) -> Self::Future {
		let fut = (self.func)();

		Box::pin(async move { fut.await.into_response_result() })
	}
}

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

		#[allow(non_snake_case)]
		impl<Func, $($T,)? Fut, O> Handler for HandlerFn<Func, (Private, $($RequestHead,)? $($T)?)>
		where
			Func: Fn($($RequestHead,)? $($T)?) -> Fut + Clone + Send + 'static,
			$($T: FromRequest,)?
			Fut: Future<Output = O> + Send,
			O: IntoResponseResult,
		{
			type Response = Response;
			type Error = BoxedErrorResponse;
			type Future = BoxedFuture<Result<Self::Response, Self::Error>>;

			#[inline(always)]
			fn handle(&self, request_context: RequestContext, _args: Args) -> Self::Future {
				let func_clone = self.func.clone();

				Box::pin(async move {
					let (request, routing_state, context_properties) = request_context.into_parts();
					let (mut head_parts, body) = request.into_parts();

					$(
						let $T = match $T::from_request(&mut head_parts, body).await {
							Ok(value) => value,
							Err(error) => return Err(error.into()),
						};
					)?

					$(
						let mut head = <$RequestHead>::new(head_parts, routing_state, context_properties);
					)?

					func_clone($(head as $RequestHead,)? $($T)?).await.into_response_result()
				})
			}
		}
	};
}

request_handler_fn!([RequestHead]);
request_handler_fn!([] T);
request_handler_fn!([RequestHead] T);

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

		#[allow(non_snake_case)]
		impl<Ext, Func, $($G,)* $($T,)? Fut, O> Handler<Body, Ext>
			for HandlerFn<Func, (Private, $($G, RequestHead,)* $($RequestHead,)? $($T,)? $($Args)?)>
		where
			Ext: Clone + Send + Sync + 'static,
			Func: Fn($($G,)* $($RequestHead,)? $($T,)? $($Args)?) -> Fut + Send + Clone + 'static,
			$($G: ExtractorGuard<Body, Ext> + Send,)*
			$($T: FromRequest,)?
			Fut: Future<Output = O> + Send,
			O: IntoResponseResult,
		{
			type Response = Response;
			type Error = BoxedErrorResponse;
			type Future = BoxedFuture<Result<Self::Response, Self::Error>>;

			#[inline(always)]
			fn handle(
				&self,
				mut request_context: RequestContext,
				mut args: Args<'_, Ext>,
			) -> Self::Future {
				let func_clone = self.func.clone();
				let args = args.to_owned();

				Box::pin(async move {
					$(
						let $G = match $G::from_request_context_and_args(&mut request_context, &args).await {
							Ok(g) => g,
							Err(error) => return Err(error.into()),
						};
					)*

					let (mut request, routing_state, context_properties) = request_context.into_parts();
					let (mut head_parts, body) = request.into_parts();

					$(
						let $T = match $T::from_request(&mut head_parts, body).await {
							Ok(value) => value,
							Err(error) => return Err(error.into()),
						};
					)?

					$(
						let mut head = <$RequestHead>::new(head_parts, routing_state, context_properties);
					)?

					func_clone($($G,)* $(head as $RequestHead,)? $($T,)? $(args as $Args,)?)
						.await
						.into_response_result()
				})
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

// --------------------------------------------------------------------------------
