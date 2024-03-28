use std::{
	any::Any,
	convert::Infallible,
	fmt::Debug,
	future::Future,
	marker::PhantomData,
	process::Output,
	sync::Arc,
	task::{Context, Poll},
};

use argan_core::{body::Body, extensions::{NodeExtension, NodeExtensions}, Args as CoreArgs, BoxedFuture};
use bytes::Bytes;
use http::{Extensions, StatusCode};
use tower_service::Service;

use crate::{
	common::{BoxedAny, Uncloneable},
	middleware::Layer,
	request::{FromRequest, FromRequestHead, Request, RequestHead},
	response::{BoxedErrorResponse, IntoResponse, Response},
	routing::RoutingState,
};

// --------------------------------------------------

pub(crate) mod futures;
use futures::{DefaultResponseFuture, ResponseToResultFuture, ResultToResponseFuture};

mod impls;
pub use impls::*;

mod kind;
pub use kind::*;

pub(crate) mod request_handlers;

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

pub trait Handler<B = Body, Ext = ()> {
	type Response;
	type Error;
	type Future: Future<Output = Result<Self::Response, Self::Error>>;

	fn handle(&self, request: Request<B>, args: &mut Args<'_, Ext>) -> Self::Future;
}

// -------------------------

impl<S, B> Handler<B> for S
where
	S: Service<Request<B>> + Clone,
	S::Response: IntoResponse,
	S::Error: Into<BoxedErrorResponse>,
{
	type Response = S::Response;
	type Error = S::Error;
	type Future = S::Future;

	fn handle(&self, mut request: Request<B>, args: &mut Args) -> Self::Future {
		let args = args.into_owned_without_handler_extensions();

		request.extensions_mut().insert(Uncloneable::from(args));

		self.clone().call(request)
	}
}

// -------------------------

pub trait IntoHandler<Mark, B = Body, Ext = ()>: Sized {
	type Handler: Handler<B, Ext>;

	fn into_handler(self) -> Self::Handler;
}

impl<H, B, Ext> IntoHandler<(), B, Ext> for H
where
	H: Handler<B, Ext>,
{
	type Handler = Self;

	fn into_handler(self) -> Self::Handler {
		self
	}
}

// --------------------------------------------------

pub trait IntoWrappedHandler<Mark>: IntoHandler<Mark> + Sized {
	fn wrapped_in<L: Layer<Self::Handler>>(self, layer: L) -> L::Handler;
}

impl<H, Mark> IntoWrappedHandler<Mark> for H
where
	H: IntoHandler<Mark>,
	H::Handler: Handler + Clone + Send + Sync + 'static,
	<H::Handler as Handler>::Response: IntoResponse,
	<H::Handler as Handler>::Error: Into<BoxedErrorResponse>,
{
	fn wrapped_in<L: Layer<H::Handler>>(self, layer: L) -> L::Handler {
		layer.wrap(self.into_handler())
	}
}

// --------------------------------------------------

pub trait IntoExtendedHandler<Mark, Ext>: IntoHandler<Mark, Body, Ext> + Sized {
	fn with_extension(self, handler_extension: Ext) -> ExtendedHandler<Self::Handler, Ext>;
}

impl<H, Mark, Ext> IntoExtendedHandler<Mark, Ext> for H
where
	H: IntoHandler<Mark, Body, Ext>,
	H::Handler: Handler + Clone + Send + Sync + 'static,
	<H::Handler as Handler>::Response: IntoResponse,
	<H::Handler as Handler>::Error: Into<BoxedErrorResponse>,
{
	fn with_extension(self, handler_extension: Ext) -> ExtendedHandler<Self::Handler, Ext> {
		ExtendedHandler::new(self.into_handler(), handler_extension)
	}
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

impl<H, B> Service<Request<B>> for HandlerService<H>
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
		let mut args = request
			.extensions_mut()
			.remove::<Uncloneable<Args>>()
			.expect("Uncloneable<Args> should be inserted in the Handler implementation for the Service")
			.into_inner()
			.expect("Uncloneable must always have a valid value");

		self.handler.handle(request, &mut args)
	}
}

// --------------------------------------------------
// ExtendedHandler

#[derive(Clone)]
pub struct ExtendedHandler<H, Ext> {
	inner: H,
	extension: Ext,
}

impl<H, Ext> ExtendedHandler<H, Ext> {
	pub fn new(inner: H, extension: Ext) -> Self {
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
	fn handle(&self, mut request: Request<B>, args: &mut Args) -> Self::Future {
		let routing_state = args.take_private_extension();
		let node_extensions = args.take_node_extensions();

		let mut args = Args::new(routing_state, &self.extension);
		args.node_extensions = node_extensions;

		self.inner.handle(request, &mut args)
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
	fn handle(&self, request: Request, args: &mut Args) -> Self::Future {
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
	fn handle(&self, request: Request, args: &mut Args) -> Self::Future {
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
	fn handle(&self, _req: Request, _args: &mut Args) -> Self::Future {
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
	fn handle(&self, _req: Request, _args: &mut Args) -> Self::Future {
		Box::pin(DefaultResponseFuture::new())
	}
}

// --------------------------------------------------------------------------------
// Args

// pub struct Args<'n, HandlerExt = ()> {
// 	pub(crate) routing_state: RoutingState,
// 	pub(crate) node_extensions: NodeExtensions<'n>,
// 	pub(crate) handler_extension: &'n HandlerExt,
// }
//
// impl Args<'_, ()> {
// 	pub(crate) fn new() -> Args<'static> {
// 		Args {
// 			routing_state: RoutingState::default(),
// 			node_extensions: NodeExtensions::new_owned(Extensions::new()),
// 			handler_extension: &(),
// 		}
// 	}
//
// 	#[inline]
// 	pub(crate) fn node_extension_replaced<'e>(
// 		&mut self,
// 		node_extensions: &'e Extensions,
// 	) -> Args<'e> {
// 		let Args {
// 			routing_state,
// 			handler_extension,
// 			..
// 		} = self;
//
// 		let mut args = Args {
// 			routing_state: std::mem::take(routing_state),
// 			node_extensions: NodeExtensions::new_borrowed(node_extensions),
// 			handler_extension: &(),
// 		};
//
// 		args
// 	}
// }
//
// impl<'n, HandlerExt> Arguments<'n, HandlerExt> for Args<'n, HandlerExt> {
// 	#[allow(refining_impl_trait)]
// 	#[inline(always)]
// 	fn private_extension(&mut self) -> &mut RoutingState {
// 		&mut self.routing_state
// 	}
//
// 	#[inline(always)]
// 	fn node_extension<Ext: Send + Sync + 'static>(&self) -> Option<&'n Ext> {
// 		self.node_extensions.get_ref::<Ext>()
// 	}
//
// 	#[inline(always)]
// 	fn handler_extension(&self) -> &'n HandlerExt {
// 		self.handler_extension
// 	}
// }

// --------------------------------------------------------------------------------
// Args

pub type Args<'n, HandlerExt = ()> = CoreArgs<'n, RoutingState, HandlerExt>;

pub(crate) trait ArgsExt<'n, HandlerExt = ()>: Sized {
	fn take_private_extension(&mut self) -> RoutingState;
	fn take_node_extensions(&mut self) -> NodeExtensions<'n>;
	fn replace_node_and_handler_extensions<'new_n, NewHandlerExt>(
		&mut self,
		new_node_extensions: NodeExtensions<'new_n>,
		new_handler_extension: &'new_n NewHandlerExt,
	) -> Args<'new_n, NewHandlerExt>;

	fn into_owned_without_handler_extensions(&mut self) -> Args<'static, ()>;
}

impl<'n, HandlerExt> ArgsExt<'n, HandlerExt> for Args<'n, HandlerExt> {
	fn take_private_extension(&mut self) -> RoutingState {
		std::mem::take(self.private_extension_mut())
	}

	fn take_node_extensions(&mut self) -> NodeExtensions<'n> {
		std::mem::replace(
			&mut self.node_extensions,
			NodeExtensions::Owned(Extensions::new()),
		)
	}

	fn replace_node_and_handler_extensions<'new_n, NewHandlerExt>(
		&mut self,
		new_node_extensions: NodeExtensions<'new_n>,
		new_handler_extension: &'new_n NewHandlerExt,
	) -> Args<'new_n, NewHandlerExt> {
		let routing_state = self.take_private_extension();

		let mut args = Args::new(routing_state, new_handler_extension);
		args.node_extensions = new_node_extensions;

		args
	}

	fn into_owned_without_handler_extensions(&mut self) -> Args<'static, ()> {
		let routing_state = self.take_private_extension();
		let mut args = Args::new(routing_state, &());
		args.node_extensions = self.take_node_extensions().into_owned();

		args
	}
}
