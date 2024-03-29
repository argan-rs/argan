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
	request::{FromRequest, FromRequestHead, Request},
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
	fn handle(&self, request: Request<B>, args: Args<'_, ()>) -> Self::Future {
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

impl<Func> IntoHandler<Request> for Func
where
	Func: Fn(Request) -> BoxedFuture<Result<Response, BoxedError>>,
	HandlerFn<Func, Request>: Handler,
{
	type Handler = HandlerFn<Func, Request>;

	fn into_handler(self) -> Self::Handler {
		HandlerFn::from(self)
	}
}

impl<Func, Ext> Handler<Body, Ext> for HandlerFn<Func, Request>
where
	Func: Fn(Request) -> BoxedFuture<Result<Response, BoxedErrorResponse>>,
{
	type Response = Response;
	type Error = BoxedErrorResponse;
	type Future = BoxedFuture<Result<Self::Response, Self::Error>>;

	#[inline(always)]
	fn handle(&self, request: Request, _args: Args<'_, Ext>) -> Self::Future {
		(self.func)(request)
	}
}

// -------------------------

impl<'r, Func, Ext> IntoHandler<(Request, Args<'r>, Ext)> for Func
where
	Func: Fn(Request, Args<'_, Ext>) -> BoxedFuture<Result<Response, BoxedError>>,
	HandlerFn<Func, (Request, Args<'r>, Ext)>: Handler,
{
	type Handler = HandlerFn<Func, (Request, Args<'r>, Ext)>;

	fn into_handler(self) -> Self::Handler {
		HandlerFn::from(self)
	}
}

impl<'r, Func, Ext> Handler<Body, Ext> for HandlerFn<Func, (Request, Args<'r>, Ext)>
where
	Func: Fn(Request, Args<'_, Ext>) -> BoxedFuture<Result<Response, BoxedErrorResponse>>,
{
	type Response = Response;
	type Error = BoxedErrorResponse;
	type Future = BoxedFuture<Result<Self::Response, Self::Error>>;

	#[inline(always)]
	fn handle(&self, request: Request, args: Args<'_, Ext>) -> Self::Future {
		(self.func)(request, args)
	}
}

// --------------------------------------------------

#[rustfmt::skip]
macro_rules! impl_handler_fn {
	($(($($ps:ident),*),)? $($lp:ident)?) => {
		#[allow(non_snake_case)]
		impl<Func, $($($ps,)*)? $($lp,)? Fut, O, B, Ext>
		IntoHandler<(Private, $($($ps,)*)? $($lp)?), B, Ext>
		for Func
		where
			Func: Fn($($($ps,)*)? $($lp)?) -> Fut,
			Fut: Future<Output = O>,
			O: IntoResponseResult,
			HandlerFn<Func, (Private, $($($ps,)*)? $($lp)?)>: Handler<B, Ext>,
		{
			type Handler = HandlerFn<Func, (Private, $($($ps,)*)? $($lp)?)>;

			fn into_handler(self) -> Self::Handler {
				HandlerFn::from(self)
			}
		}

		#[allow(non_snake_case)]
		impl<Func, $($($ps,)*)? $($lp,)? Fut, O, B, Ext> Handler<B, Ext>
		for HandlerFn<Func, (Private, $($($ps,)*)? $($lp)?)>
		where
			Func: Fn($($($ps,)*)? $($lp)?) -> Fut + Clone + 'static,
			$($($ps: for <'n> FromRequestHead<Args<'n, Ext>>,)*)?
			$($lp: for <'n> FromRequest<B, Args<'n, Ext>>,)?
			Fut: Future<Output = O>,
			O: IntoResponseResult,
			B: 'static,
			Ext: Clone + Sync + 'static,
		{
			type Response = Response;
			type Error = BoxedErrorResponse;
			type Future = HandlerFnFuture<Func, (Private, $($($ps,)*)? $($lp)?), B, Ext>;

			fn handle(&self, request: Request<B>, mut args: Args<'_, Ext>) -> Self::Future {
				let func_clone = self.func.clone();
				let routing_state = args.take_routing_state();
				let node_extensions = args.take_node_extensions().into_owned();
				let handler_extension_clone = args.handler_extension.clone();

				HandlerFnFuture::new(
					func_clone,
					request,
					routing_state,
					node_extensions,
					handler_extension_clone,
				)
			}
		}

		#[allow(non_snake_case)]
		impl<Func, $($($ps,)*)? $($lp,)? Fut, O, B, Ext> Future
		for HandlerFnFuture<Func, (Private, $($($ps,)*)? $($lp)?), B, Ext>
		where
			Func: Fn($($($ps,)*)? $($lp)?) -> Fut + Clone + 'static,
			$($($ps: for <'n> FromRequestHead<Args<'n, Ext>>,)*)?
			$($lp: for <'n> FromRequest<B, Args<'n, Ext>>,)?
			Fut: Future<Output = O>,
			O: IntoResponseResult,
			B: 'static,
			Ext: Sync,
		{
			type Output = Result<Response, BoxedErrorResponse>;

			fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
				let self_projection = self.project();

				let routing_state = self_projection.some_routing_state.take().expect(
					"HandlerFnFuture should be created with a routing state",
				);

				let node_extensions = self_projection.some_node_extensions.take().expect(
					"HandlerFnFuture should be created with the node extensions",
				);

				let handler_extension = self_projection.some_handler_extension.take().expect(
					"HandlerFnFuture should be created with a handler extension",
				);

				let mut args = Args {
					routing_state,
					node_extensions,
					handler_extension: &handler_extension,
				};
			
				$(
					let (mut head, body) = self_projection.some_request.take().expect(
						"HandlerFnFuture should be created with a request",
					).into_parts();

					$(
						let $ps = match pin!($ps::from_request_head(&mut head, &mut args)).poll(cx) {
							Poll::Ready(result) => {
								match result {
									Ok(value) => value,
									Err(error) => return Poll::Ready(Err(error.into())),
								}
							},
							Poll::Pending => return Poll::Pending,
						};
					)*

					self_projection.some_request.replace(Request::<B>::from_parts(head, body));
				)?

				$(
					let $lp =
						match pin!(
							$lp::from_request(
								self_projection.some_request.take().expect(
									"the constructor of the HandlerFnFuture or the local scope should set the request"
								),
								args,
							)
						).poll(cx) {
							Poll::Ready(result) => {
								match result {
									Ok(value) => value,
									Err(error) => return Poll::Ready(Err(error.into())),
								}
							}
							Poll::Pending => return Poll::Pending,
						};
				)?

				match pin!((self_projection.func)($($($ps,)*)? $($lp)?)).poll(cx) {
					Poll::Ready(value) => Poll::Ready(value.into_response_result()),
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
#[rustfmt::skip]
impl_handler_fn!((P1, P2, P3, P4, P5, P6, P7, P8, P9, P10, P11, P12, P13, P14), LP);
#[rustfmt::skip]
impl_handler_fn!((P1, P2, P3, P4, P5, P6, P7, P8, P9, P10, P11, P12, P13, P14, P15), LP);

// --------------------------------------------------

#[pin_project]
pub struct HandlerFnFuture<Func, Mark, B, E: 'static> {
	func: Func,
	some_request: Option<Request<B>>,
	some_routing_state: Option<RoutingState>,
	some_node_extensions: Option<NodeExtensions<'static>>,
	some_handler_extension: Option<E>,
	_mark: PhantomData<fn() -> Mark>,
}

impl<Func, Mark, B, E> HandlerFnFuture<Func, Mark, B, E> {
	fn new(
		func: Func,
		request: Request<B>,
		routing_state: RoutingState,
		node_extensions: NodeExtensions<'static>,
		handler_extension: E,
	) -> Self {
		Self {
			func,
			some_request: Some(request),
			some_routing_state: Some(routing_state),
			some_node_extensions: Some(node_extensions),
			some_handler_extension: Some(handler_extension),
			_mark: PhantomData,
		}
	}
}
