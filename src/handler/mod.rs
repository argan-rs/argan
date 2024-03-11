use std::{
	convert::Infallible, fmt::Debug, future::Future, marker::PhantomData, process::Output, sync::Arc,
};

use crate::{
	body::{Body, HttpBody},
	common::{BoxedError, BoxedFuture, Uncloneable},
	data::extensions::NodeExtensions,
	middleware::Layer,
	request::{FromRequest, FromRequestHead, Request, RequestHead},
	response::{BoxedErrorResponse, IntoResponse, Response},
	routing::RoutingState,
};

// ----------

use bytes::Bytes;
use http::{Extensions, StatusCode};
pub use hyper::service::Service;

// --------------------------------------------------

pub(crate) mod futures;
mod impls;
mod kind;
pub(crate) mod request_handlers;

use self::futures::{DefaultResponseFuture, ResponseToResultFuture, ResultToResponseFuture};
pub(crate) use impls::*;
pub use kind::*;

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

pub trait Handler<B = Body, Ext = ()> {
	type Response;
	type Error;
	type Future: Future<Output = Result<Self::Response, Self::Error>>;

	fn handle(&self, request: Request<B>, args: &mut Args<'_, Ext>) -> Self::Future;
}

// impl<S, B> Handler<B> for S
// where
// 	S: Service<Request<B>>,
// 	S::Error: Into<BoxedError>
// {
// 	type Response = S::Response;
// 	type Error = BoxedError;
// 	type Future = BoxedFuture<Result<Self::Response, Self::Error>>;
//
// 	fn handle(&self, mut request: Request<B>, args: &mut Args) -> Self::Future {
// 		request
// 			.extensions_mut()
// 			.insert(args.node_extensions.clone().into_owned()); // ???
//
// 		self.call(request).map(Into::into)
// 		// let result_future = self.call(request);
// 		//
// 		// ResultToResponseFuture::from(result_future)
// 	}
// }

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

	#[inline]
	fn call(&self, mut request: Request<B>) -> Self::Future {
		let routing_state = request
			.extensions_mut()
			.remove::<Uncloneable<RoutingState>>()
			.expect("Uncloneable<RoutingState> should be inserted before routing started")
			.into_inner()
			.expect("RoutingState should always exist in Uncloneable");

		let node_extensions = request
			.extensions_mut()
			.remove::<NodeExtensions>()
			.expect("when layered, resource extensions must be inserted into the request");

		let mut args = Args {
			routing_state,
			node_extensions,
			handler_extension: &(),
		};

		self.handler.handle(request, &mut args)
		// let response_future = self.handler.handle(request, &mut args);
		//
		// ResponseToResultFuture::from(response_future)
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
		let routing_state = std::mem::take(&mut args.routing_state);
		let node_extensions = args.node_extensions.take();

		let mut args = Args {
			routing_state,
			node_extensions,
			handler_extension: &self.extension,
		};

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
		Self(Arc::new(DummyHandler::<BoxedFuture<Result<Response, BoxedErrorResponse>>>::new()))
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

#[non_exhaustive]
pub struct Args<'r, Ext = ()> {
	pub(crate) routing_state: RoutingState,
	pub node_extensions: NodeExtensions<'r>,
	pub handler_extension: &'r Ext, // The handler has the same lifetime as the resource it belongs to.
}

impl Args<'_, ()> {
	pub(crate) fn default() -> Args<'static> {
		Args {
			routing_state: RoutingState::default(),
			node_extensions: NodeExtensions::new_owned(Extensions::new()),
			handler_extension: &(),
		}
	}

	#[inline]
	pub(crate) fn node_extensions_replaced<'e>(&mut self, extensions: &'e Extensions) -> Args<'e> {
		let Args {
			routing_state,
			node_extensions,
			handler_extension,
		} = self;

		let mut args = Args {
			routing_state: std::mem::take(routing_state),
			node_extensions: NodeExtensions::new_borrowed(extensions),
			handler_extension: &(),
		};

		args
	}
}

// --------------------------------------------------------------------------------
