use std::{convert::Infallible, fmt::Debug, future::Future, marker::PhantomData};

pub use hyper::service::Service;

use crate::{
	body::Body,
	body::IncomingBody,
	middleware::{IntoResponseAdapter, Layer, RequestBodyAdapter, ResponseFutureBoxer},
	request::Request,
	response::{IntoResponse, Response},
	utils::{BoxedError, BoxedFuture},
};

// --------------------------------------------------

pub(crate) mod futures;
use self::futures::*;

pub mod impls;
pub(crate) mod request_handlers;

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

pub trait IntoHandler<M, B = IncomingBody>: Sized {
	type Handler: Handler<B>;

	fn into_handler(self) -> Self::Handler;

	// --------------------------------------------------
	// Provided Methods

	fn with_state<S>(self, state: S) -> StatefulHandler<Self::Handler, S> {
		StatefulHandler::new(self.into_handler(), state)
	}

	fn wrapped_in<L, NewB>(self, layer: L) -> L::Handler
	where
		L: Layer<Self::Handler, NewB>,
	{
		layer.wrap(self.into_handler())
	}
}

impl<H, B> IntoHandler<Request<B>, B> for H
where
	H: Handler<B>,
{
	type Handler = Self;

	fn into_handler(self) -> Self::Handler {
		self
	}
}

// --------------------------------------------------------------------------------

pub struct HandlerService<H, B> {
	handler: H,
	_body_mark: PhantomData<B>,
}

impl<H, B> From<H> for HandlerService<H, B> {
	#[inline]
	fn from(handler: H) -> Self {
		Self {
			handler,
			_body_mark: PhantomData,
		}
	}
}

impl<H, B> Service<Request<B>> for HandlerService<H, B>
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

pub struct HandlerState<S>(S);

// --------------------------------------------------------------------------------

pub(crate) trait ReadyHandler
where
	Self: Handler<IncomingBody, Response = Response, Future = BoxedFuture<Response>> + Sync,
{
}

impl<H> ReadyHandler for H where
	H: Handler<IncomingBody, Response = Response, Future = BoxedFuture<Response>> + Sync
{
}

// --------------------------------------------------

pub(crate) type BoxedHandler = Box<dyn ReadyHandler>;

impl Handler<IncomingBody> for BoxedHandler {
	type Response = Response;
	type Future = BoxedFuture<Self::Response>;

	#[inline]
	fn handle(&self, request: Request<IncomingBody>) -> Self::Future {
		self.as_ref().handle(request)
	}
}

impl Default for BoxedHandler {
	#[inline]
	fn default() -> Self {
		Box::new(DummyHandler::<BoxedFuture<Response>>::new())
	}
}

pub(crate) fn wrap_boxed_handler<L, LayeredB>(boxed_handler: BoxedHandler, layer: L) -> BoxedHandler
where
	L: Layer<AdaptiveHandler<LayeredB>, LayeredB>,
	L::Handler: Handler<IncomingBody> + Sync + 'static,
	<L::Handler as Handler<IncomingBody>>::Response: IntoResponse,
{
	let adaptive_handler = AdaptiveHandler::from(RequestBodyAdapter::wrap(boxed_handler));
	let layered_handler = layer.wrap(adaptive_handler);
	let ready_handler = ResponseFutureBoxer::wrap(IntoResponseAdapter::wrap(layered_handler));

	ready_handler.into_boxed_handler()
}

// --------------------------------------------------

pub(crate) struct DummyHandler<M> {
	_mark: PhantomData<fn() -> M>,
}

impl DummyHandler<DefaultResponseFuture> {
	pub(crate) fn new() -> Self {
		Self { _mark: PhantomData }
	}
}

impl Handler<IncomingBody> for DummyHandler<DefaultResponseFuture> {
	type Response = Response;
	type Future = DefaultResponseFuture;

	#[inline]
	fn handle(&self, req: Request<IncomingBody>) -> Self::Future {
		DefaultResponseFuture::new()
	}
}

impl DummyHandler<BoxedFuture<Response>> {
	pub(crate) fn new() -> Self {
		Self { _mark: PhantomData }
	}
}

impl Handler<IncomingBody> for DummyHandler<BoxedFuture<Response>> {
	type Response = Response;
	type Future = BoxedFuture<Response>;

	#[inline]
	fn handle(&self, req: Request<IncomingBody>) -> Self::Future {
		Box::pin(DefaultResponseFuture::new())
	}
}

// --------------------------------------------------

pub struct AdaptiveHandler<B> {
	inner: RequestBodyAdapter<BoxedHandler>,
	_body: PhantomData<B>,
}

impl<B> Service<Request<B>> for AdaptiveHandler<B>
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
		let response_future = self.inner.handle(request);

		ResponseToResultFuture::from(response_future)
	}
}

impl<B> From<RequestBodyAdapter<BoxedHandler>> for AdaptiveHandler<B> {
	#[inline]
	fn from(handler: RequestBodyAdapter<BoxedHandler>) -> Self {
		Self {
			inner: handler,
			_body: PhantomData,
		}
	}
}

// --------------------------------------------------------------------------------
