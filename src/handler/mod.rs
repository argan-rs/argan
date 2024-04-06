use std::{
	any::Any,
	borrow::Cow,
	convert::Infallible,
	fmt::Debug,
	future::Future,
	marker::PhantomData,
	process::Output,
	sync::Arc,
	task::{Context, Poll},
};

use argan_core::{body::Body, BoxedFuture};
use bytes::Bytes;
use futures_util::FutureExt;
use http::{Extensions, Request, StatusCode};
use tower_service::Service as TowerService;

use crate::{
	common::{BoxedAny, Uncloneable},
	data::extensions::NodeExtensions,
	middleware::Layer,
	request::{RequestContext, RequestHead},
	response::{BoxedErrorResponse, IntoResponse, Response},
	routing::RoutingState,
};

// --------------------------------------------------

pub(crate) mod futures;
use futures::{DefaultResponseFuture, ResponseToResultFuture, ResultToResponseFuture};

mod impls;
pub(crate) use impls::*;

mod kind;
pub use kind::*;

pub(crate) mod request_handlers;

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

pub trait Handler<B = Body, Ext: Clone = ()> {
	type Response;
	type Error;
	type Future: Future<Output = Result<Self::Response, Self::Error>>;

	fn handle(&self, request: RequestContext<B>, args: Args<'_, Ext>) -> Self::Future;
}

// -------------------------

impl<S, B> Handler<B> for S
where
	S: TowerService<Request<B>> + Clone,
	S::Response: IntoResponse,
	S::Error: Into<BoxedErrorResponse>,
{
	type Response = S::Response;
	type Error = S::Error;
	type Future = S::Future;

	fn handle(&self, mut request: RequestContext<B>, mut args: Args) -> Self::Future {
		let (mut request, routing_state, some_cookie_key) = request.into_parts();
		let args = args.to_owned();

		request
			.extensions_mut()
			.insert(Uncloneable::from((routing_state, args, some_cookie_key)));

		self.clone().call(request)
	}
}

// --------------------------------------------------
// IntoHandler

pub trait IntoHandler<Mark, B = Body, Ext: Clone = ()>: Sized {
	type Handler: Handler<B, Ext>;

	fn into_handler(self) -> Self::Handler;

	fn with_error_handler<E: ErrorHandler<<Self::Handler as Handler<B, Ext>>::Error>>(
		self,
		error_handler: E,
	) -> ResponseResultHandler<Self::Handler, E> {
		ResponseResultHandler::new(self.into_handler(), error_handler)
	}

	fn with_extension(self, handler_extension: Ext) -> ExtendedHandler<Self::Handler, Ext> {
		ExtendedHandler::new(self.into_handler(), handler_extension)
	}

	fn with_cookie_key(self, cookie_key: cookie::Key) -> ContextProviderHandler<Self::Handler> {
		let handler_context = HandlerContext { cookie_key };

		ContextProviderHandler::new(self.into_handler(), handler_context)
	}

	fn wrapped_in<L: Layer<Self::Handler>>(self, layer: L) -> L::Handler {
		layer.wrap(self.into_handler())
	}
}

impl<H, B, Ext> IntoHandler<(), B, Ext> for H
where
	H: Handler<B, Ext>,
	Ext: Clone,
{
	type Handler = Self;

	fn into_handler(self) -> Self::Handler {
		self
	}
}

// --------------------------------------------------
// ErrorHandler

pub trait ErrorHandler<E> {
	// ??? We may have a problem with shared ref.
	fn handle_error(&self, error: E) -> impl Future<Output = Result<Response, E>> + Send;
}

impl<Func, Fut, E> ErrorHandler<E> for Func
where
	Func: Fn(E) -> Fut,
	Fut: Future<Output = Result<Response, E>>,
{
	fn handle_error(&self, error: E) -> impl Future<Output = Result<Response, E>> + Send {
		self(error)
	}
}

// -------------------------
// ResponseResultHandler

#[derive(Clone)]
pub struct ResponseResultHandler<H, E> {
	inner: H,
	error_handler: E,
}

impl<H, E> ResponseResultHandler<H, E> {
	pub(crate) fn new(inner: H, error_handler: E) -> Self {
		Self {
			inner,
			error_handler,
		}
	}
}

impl<H, B, Ext, E> Handler<B, Ext> for ResponseResultHandler<H, E>
where
	H: Handler<B, Ext>,
	Ext: Clone,
	E: ErrorHandler<H::Error>,
{
	type Response = H::Response;
	type Error = H::Error;
	type Future = H::Future;

	#[inline]
	fn handle(&self, mut request: RequestContext<B>, mut args: Args<'_, Ext>) -> Self::Future {
		self.inner.handle(request, args).map(|result| match result {
			Ok(response) => Ok(response),
			Err(error) => self.error_handler(error),
		})
	}
}

// --------------------------------------------------
// IntoWrappedHandler

// pub trait IntoWrappedHandler<Mark>: IntoHandler<Mark> + Sized {
// 	fn wrapped_in<L: Layer<Self::Handler>>(self, layer: L) -> L::Handler;
// }
//
// impl<H, Mark> IntoWrappedHandler<Mark> for H
// where
// 	H: IntoHandler<Mark>,
// {
// 	fn wrapped_in<L: Layer<H::Handler>>(self, layer: L) -> L::Handler {
// 		layer.wrap(self.into_handler())
// 	}
// }

// --------------------------------------------------
// IntoExtendedHandler

// pub trait IntoExtendedHandler<Mark, B, Ext: Clone>: IntoHandler<Mark, B, Ext> + Sized {
// 	fn with_extension(self, handler_extension: Ext) -> ExtendedHandler<Self::Handler, Ext>;
// }
//
// impl<H, Mark, B, Ext> IntoExtendedHandler<Mark, B, Ext> for H
// where
// 	H: IntoHandler<Mark, B, Ext>,
// 	Ext: Clone + Send + Sync + 'static,
// {
// 	fn with_extension(self, handler_extension: Ext) -> ExtendedHandler<Self::Handler, Ext> {
// 		ExtendedHandler::new(self.into_handler(), handler_extension)
// 	}
// }

// --------------------------------------------------
// ExtendedHandler

#[derive(Clone)]
pub struct ExtendedHandler<H, Ext> {
	inner: H,
	extension: Ext,
}

impl<H, Ext> ExtendedHandler<H, Ext> {
	pub(crate) fn new(inner: H, extension: Ext) -> Self {
		Self { inner, extension }
	}
}

impl<H, B, Ext> Handler<B> for ExtendedHandler<H, Ext>
where
	H: Handler<B, Ext>,
	Ext: Clone + Send + Sync + 'static,
{
	type Response = H::Response;
	type Error = H::Error;
	type Future = H::Future;

	#[inline]
	fn handle(&self, mut request: RequestContext<B>, mut args: Args) -> Self::Future {
		let node_extensions = args.take_node_extensions();

		let mut args = Args {
			node_extensions,
			handler_extension: Cow::Borrowed(&self.extension),
		};

		self.inner.handle(request, args)
	}
}

// --------------------------------------------------
// YummyHandler

pub struct ContextProviderHandler<H> {
	inner: H,
	context: HandlerContext,
}

impl<H> ContextProviderHandler<H> {
	pub(crate) fn new(handler: H, context: HandlerContext) -> Self {
		Self {
			inner: handler,
			context,
		}
	}
}

impl<H, B, Ext> Handler<B, Ext> for ContextProviderHandler<H>
where
	H: Handler<B, Ext>,
	Ext: Clone,
{
	type Response = H::Response;
	type Error = H::Error;
	type Future = H::Future;

	#[inline]
	fn handle(&self, mut request: RequestContext<B>, mut args: Args<'_, Ext>) -> Self::Future {
		let request = request.with_cookie_key(self.context.cookie_key.clone());

		self.inner.handle(request, args)
	}
}

// -------------------------
// HandlerContext

pub(crate) struct HandlerContext {
	pub(crate) cookie_key: cookie::Key,
}

// --------------------------------------------------
// HandlerService

pub struct HandlerService<H> {
	handler: H,
}

impl<H> From<H> for HandlerService<H> {
	#[inline]
	fn from(handler: H) -> Self {
		Self { handler }
	}
}

impl<H, B> TowerService<Request<B>> for HandlerService<H>
where
	H: Handler<B>,
{
	type Response = H::Response;
	type Error = H::Error;
	type Future = H::Future;

	fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
		Poll::Ready(Ok(()))
	}

	#[inline]
	fn call(&mut self, mut request: Request<B>) -> Self::Future {
		let (routing_state, args, some_cookie_key) = request
			.extensions_mut()
			.remove::<Uncloneable<(RoutingState, Args, Option<cookie::Key>)>>()
			.expect(
				"Uncloneable<(RoutingState, Args, Option<cookie::Key>)> should be inserted in the Handler implementation for the Service",
			)
			.into_inner()
			.expect("Uncloneable must always have a valid value");

		let request = RequestContext::new(request, routing_state);

		self.handler.handle(request, args)
	}
}

// --------------------------------------------------------------------------------
// FinalHandler trait

trait FinalHandler
where
	Self: Handler<
		Response = Response,
		Error = BoxedErrorResponse,
		Future = BoxedFuture<Result<Response, BoxedErrorResponse>>,
	>,
{
	fn into_boxed_handler(self) -> BoxedHandler;
	fn boxed_clone(&self) -> BoxedHandler;
}

impl<H> FinalHandler for H
where
	H: Handler<
			Response = Response,
			Error = BoxedErrorResponse,
			Future = BoxedFuture<Result<Response, BoxedErrorResponse>>,
		> + Clone
		+ Send
		+ Sync
		+ 'static,
{
	fn into_boxed_handler(self) -> BoxedHandler {
		BoxedHandler::new(self)
	}

	fn boxed_clone(&self) -> BoxedHandler {
		BoxedHandler::new(self.clone())
	}
}

// --------------------------------------------------
// BoxedHandler

pub(crate) struct BoxedHandler(Box<dyn FinalHandler + Send + Sync>);

impl BoxedHandler {
	#[allow(private_bounds)]
	#[inline(always)]
	pub(crate) fn new<H: FinalHandler + Send + Sync + 'static>(handler: H) -> Self {
		Self(Box::new(handler))
	}
}

impl Default for BoxedHandler {
	#[inline(always)]
	fn default() -> Self {
		Self(Box::new(DummyHandler::<
			BoxedFuture<Result<Response, BoxedErrorResponse>>,
		>::new()))
	}
}

impl Clone for BoxedHandler {
	#[inline(always)]
	fn clone(&self) -> Self {
		self.0.boxed_clone()
	}
}

impl Handler for BoxedHandler {
	type Response = Response;
	type Error = BoxedErrorResponse;
	type Future = BoxedFuture<Result<Self::Response, Self::Error>>;

	#[inline(always)]
	fn handle(&self, request: RequestContext, args: Args) -> Self::Future {
		self.0.handle(request, args)
	}
}

// --------------------------------------------------
// ArcHandler

#[derive(Clone)]
pub(crate) struct ArcHandler(Arc<dyn FinalHandler + Send + Sync>);

impl ArcHandler {
	#[allow(private_bounds)]
	#[inline(always)]
	pub(crate) fn new<H: FinalHandler + Send + Sync + 'static>(handler: H) -> Self {
		Self(Arc::new(handler))
	}
}

impl Default for ArcHandler {
	#[inline(always)]
	fn default() -> Self {
		Self(Arc::new(DummyHandler::<
			BoxedFuture<Result<Response, BoxedErrorResponse>>,
		>::new()))
	}
}

impl From<BoxedHandler> for ArcHandler {
	fn from(boxed_handler: BoxedHandler) -> Self {
		Self(boxed_handler.0.into())
	}
}

impl Handler for ArcHandler {
	type Response = Response;
	type Error = BoxedErrorResponse;
	type Future = BoxedFuture<Result<Self::Response, Self::Error>>;

	#[inline(always)]
	fn handle(&self, request: RequestContext, args: Args) -> Self::Future {
		self.0.handle(request, args)
	}
}

// --------------------------------------------------------------------------------
// DummyHandler

pub(crate) struct DummyHandler<F> {
	_future_mark: PhantomData<fn() -> F>,
}

impl DummyHandler<DefaultResponseFuture> {
	pub(crate) fn new() -> Self {
		Self {
			_future_mark: PhantomData,
		}
	}
}

impl Clone for DummyHandler<DefaultResponseFuture> {
	fn clone(&self) -> Self {
		Self {
			_future_mark: PhantomData,
		}
	}
}

impl Handler for DummyHandler<DefaultResponseFuture> {
	type Response = Response;
	type Error = BoxedErrorResponse;
	type Future = DefaultResponseFuture;

	#[inline(always)]
	fn handle(&self, _req: RequestContext, _args: Args) -> Self::Future {
		DefaultResponseFuture::new()
	}
}

impl DummyHandler<BoxedFuture<Result<Response, BoxedErrorResponse>>> {
	pub(crate) fn new() -> Self {
		Self {
			_future_mark: PhantomData,
		}
	}
}

impl Clone for DummyHandler<BoxedFuture<Result<Response, BoxedErrorResponse>>> {
	fn clone(&self) -> Self {
		Self {
			_future_mark: PhantomData,
		}
	}
}

impl Handler for DummyHandler<BoxedFuture<Result<Response, BoxedErrorResponse>>> {
	type Response = Response;
	type Error = BoxedErrorResponse;
	type Future = BoxedFuture<Result<Self::Response, Self::Error>>;

	#[inline(always)]
	fn handle(&self, _req: RequestContext, _args: Args) -> Self::Future {
		Box::pin(DefaultResponseFuture::new())
	}
}

// --------------------------------------------------
// Args

#[non_exhaustive]
pub struct Args<'n, HandlerExt: Clone = ()> {
	pub node_extensions: NodeExtensions<'n>,
	pub handler_extension: Cow<'n, HandlerExt>,
}

impl<'n> Args<'n, ()> {
	pub(crate) fn new() -> Args<'static, ()> {
		Args {
			node_extensions: NodeExtensions::new_owned(Extensions::new()),
			handler_extension: Cow::Borrowed(&()),
		}
	}
}

impl<'n, HandlerExt: Clone> Args<'n, HandlerExt> {
	pub(crate) fn to_owned(&mut self) -> Args<'static, HandlerExt> {
		Args {
			node_extensions: self.take_node_extensions().to_owned(),
			handler_extension: Cow::Owned(self.handler_extension.clone().into_owned()),
		}
	}

	pub(crate) fn take_node_extensions(&mut self) -> NodeExtensions<'n> {
		std::mem::replace(
			&mut self.node_extensions,
			NodeExtensions::new_owned(Extensions::new()),
		)
	}

	pub(crate) fn extensions_replaced<'new_n, NewHandlerExt: Clone>(
		&mut self,
		new_node_extensions: NodeExtensions<'new_n>,
		new_handler_extension: &'new_n NewHandlerExt,
	) -> Args<'new_n, NewHandlerExt> {
		Args {
			node_extensions: new_node_extensions,
			handler_extension: Cow::Borrowed(new_handler_extension),
		}
	}
}

// --------------------------------------------------------------------------------
