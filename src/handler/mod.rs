use std::{convert::Infallible, fmt::Debug, future::Future, marker::PhantomData, sync::Arc};

use crate::{
	body::Body,
	body::IncomingBody,
	middleware::{IntoResponseAdapter, Layer, RequestBodyAdapter, ResponseFutureBoxer},
	request::Request,
	response::{IntoResponse, Response},
	utils::{BoxedError, BoxedFuture},
};

// ----------

pub use hyper::service::Service;

// --------------------------------------------------

pub(crate) mod futures;
mod impls;
mod kind;
pub(crate) mod request_handlers;

use self::futures::{DefaultResponseFuture, ResponseToResultFuture, ResultToResponseFuture};
pub use impls::*;
pub use kind::*;

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

pub trait Handler<B = IncomingBody> {
	type Response;
	type Future: Future<Output = Self::Response>;

	fn handle(&self, request: Request<B>) -> Self::Future;
}

impl<S, B> Handler<B> for S
where
	S: Service<Request<B>, Error = Infallible>,
{
	type Response = S::Response;
	type Future = ResultToResponseFuture<S::Future>;

	fn handle(&self, request: Request<B>) -> Self::Future {
		let result_future = self.call(request);

		ResultToResponseFuture::from(result_future)
	}
}

// -------------------------

pub trait IntoHandler<Mark, B = IncomingBody>: Sized {
	type Handler: Handler<B>;

	fn into_handler(self) -> Self::Handler;

	// --------------------------------------------------
	// Provided Methods

	fn with_state<S>(self, state: S) -> StatefulHandler<Self::Handler, S> {
		StatefulHandler::new(self.into_handler(), state)
	}

	fn wrapped_in<L>(self, layer: L) -> L::Handler
	where
		L: Layer<Self::Handler>,
	{
		layer.wrap(self.into_handler())
	}
}

impl<H, B> IntoHandler<(), B> for H
where
	H: Handler<B>,
{
	type Handler = Self;

	fn into_handler(self) -> Self::Handler {
		self
	}
}

// --------------------------------------------------------------------------------

pub struct HandlerService<H> {
	handler: H,
	// _body_mark: PhantomData<B>,
}

impl<H> From<H> for HandlerService<H> {
	#[inline]
	fn from(handler: H) -> Self {
		Self {
			handler,
			// _body_mark: PhantomData,
		}
	}
}

impl<H, B> Service<Request<B>> for HandlerService<H>
where
	H: Handler<B>,
{
	type Response = H::Response;
	type Error = Infallible;
	type Future = ResponseToResultFuture<H::Future>;

	#[inline]
	fn call(&self, request: Request<B>) -> Self::Future {
		let response_future = self.handler.handle(request);

		ResponseToResultFuture::from(response_future)
	}
}

// -------------------------

pub struct StatefulHandler<H, S> {
	inner: H,
	state: S,
}

impl<H, S> StatefulHandler<H, S> {
	pub fn new(inner: H, state: S) -> Self {
		Self { inner, state }
	}
}

impl<H, B, S> Handler<B> for StatefulHandler<H, S>
where
	H: Handler<B>,
	S: Clone + Send + Sync + 'static,
{
	type Response = H::Response;
	type Future = H::Future;

	#[inline]
	fn handle(&self, mut request: Request<B>) -> Self::Future {
		if let Some(_previous_state_with_the_same_type) = request
			.extensions_mut()
			.insert(HandlerState(self.state.clone()))
		{
			panic!("state with the same type exists")
		}

		self.inner.handle(request)
	}
}

// ----------

#[derive(Clone)]
pub struct HandlerState<S>(S);

// --------------------------------------------------------------------------------

pub(crate) trait ReadyHandler
where
	Self: Handler<IncomingBody, Response = Response, Future = BoxedFuture<Response>> + Send + Sync,
{
}

impl<H> ReadyHandler for H where
	H: Handler<IncomingBody, Response = Response, Future = BoxedFuture<Response>> + Send + Sync
{
}

pub(crate) trait IntoArcHandler: ReadyHandler + Sized + 'static {
	fn into_arc_handler(self) -> ArcHandler {
		ArcHandler::new(self)
	}
}

impl<H> IntoArcHandler for H where H: ReadyHandler + 'static {}

// --------------------------------------------------

#[derive(Clone)]
pub(crate) struct ArcHandler(Arc<dyn ReadyHandler>);

impl Default for ArcHandler {
	#[inline]
	fn default() -> Self {
		ArcHandler(Arc::new(DummyHandler::<BoxedFuture<Response>>::new()))
	}
}

impl ArcHandler {
	fn new<H: ReadyHandler + 'static>(handler: H) -> Self {
		ArcHandler(Arc::new(handler))
	}
}

impl Handler for ArcHandler {
	type Response = Response;
	type Future = BoxedFuture<Self::Response>;

	#[inline]
	fn handle(&self, request: Request<IncomingBody>) -> Self::Future {
		self.0.handle(request)
	}
}

// --------------------------------------------------

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

impl Handler for DummyHandler<DefaultResponseFuture> {
	type Response = Response;
	type Future = DefaultResponseFuture;

	#[inline]
	fn handle(&self, _req: Request<IncomingBody>) -> Self::Future {
		DefaultResponseFuture::new()
	}
}

impl DummyHandler<BoxedFuture<Response>> {
	pub(crate) fn new() -> Self {
		Self {
			_future_mark: PhantomData,
		}
	}
}

impl Handler for DummyHandler<BoxedFuture<Response>> {
	type Response = Response;
	type Future = BoxedFuture<Response>;

	#[inline]
	fn handle(&self, _req: Request<IncomingBody>) -> Self::Future {
		Box::pin(DefaultResponseFuture::new())
	}
}

// --------------------------------------------------

pub struct AdaptiveHandler(RequestBodyAdapter<ArcHandler>);

impl<B> Service<Request<B>> for AdaptiveHandler
where
	B: Body + Send + Sync + 'static,
	B::Data: Debug,
	B::Error: Into<BoxedError>,
{
	type Response = Response;
	type Error = Infallible;
	type Future = ResponseToResultFuture<BoxedFuture<Response>>;

	#[inline]
	fn call(&self, request: Request<B>) -> Self::Future {
		let response_future = self.0.handle(request);

		ResponseToResultFuture::from(response_future)
	}
}

impl From<ArcHandler> for AdaptiveHandler {
	#[inline]
	fn from(arc_handler: ArcHandler) -> Self {
		Self(RequestBodyAdapter::wrap(arc_handler))
	}
}

// --------------------------------------------------------------------------------
