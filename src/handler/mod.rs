use std::{convert::Infallible, fmt::Debug, future::Future, marker::PhantomData, sync::Arc};

use crate::{
	body::{Body, HttpBody},
	common::{BoxedError, BoxedFuture, Uncloneable},
	middleware::Layer,
	request::{FromRequest, FromRequestHead, Request, RequestHead},
	resource::ResourceExtensions,
	response::Response,
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
pub use impls::*;
pub use kind::*;

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

pub trait Handler<B = Body, E = ()> {
	type Response;
	type Future: Future<Output = Self::Response>;

	fn handle(&self, request: Request<B>, args: &mut Args<'_, E>) -> Self::Future;
}

impl<S, B> Handler<B> for S
where
	S: Service<Request<B>, Error = Infallible>,
{
	type Response = S::Response;
	type Future = ResultToResponseFuture<S::Future>;

	fn handle(&self, mut request: Request<B>, args: &mut Args) -> Self::Future {
		request
			.extensions_mut()
			.insert(args.resource_extensions.clone().into_owned()); // ???

		let result_future = self.call(request);

		ResultToResponseFuture::from(result_future)
	}
}

// -------------------------

pub trait IntoHandler<Mark, B = Body, E = ()>: Sized {
	type Handler: Handler<B, E>;

	fn into_handler(self) -> Self::Handler;

	// --------------------------------------------------
	// Provided Methods

	fn with_extension(self, handler_extension: E) -> ExtendedHandler<Self::Handler, E> {
		ExtendedHandler::new(self.into_handler(), handler_extension)
	}

	fn wrap_with<L>(self, layer: L) -> L::Handler
	where
		L: Layer<Self::Handler>,
	{
		layer.wrap(self.into_handler())
	}
}

impl<H, B, E> IntoHandler<(), B, E> for H
where
	H: Handler<B, E>,
{
	type Handler = Self;

	fn into_handler(self) -> Self::Handler {
		self
	}
}

// -------------------------
// Args

#[non_exhaustive]
pub struct Args<'r, E = ()> {
	pub(crate) routing_state: RoutingState,
	pub resource_extensions: ResourceExtensions<'r>,
	pub handler_extension: &'r E, // The handler has the same lifetime as the resource it belongs to.
}

impl Args<'_, ()> {
	#[inline]
	pub(crate) fn resource_extensions_replaced<'e>(
		&mut self,
		extensions: &'e Extensions,
	) -> Args<'e> {
		let Args {
			routing_state,
			resource_extensions,
			handler_extension,
		} = self;

		let mut args = Args {
			routing_state: std::mem::take(routing_state),
			resource_extensions: ResourceExtensions::new_borrowed(extensions),
			handler_extension: &(),
		};

		args
	}
}

// --------------------------------------------------------------------------------

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
	type Error = Infallible;
	type Future = ResponseToResultFuture<H::Future>;

	#[inline]
	fn call(&self, mut request: Request<B>) -> Self::Future {
		let routing_state = request
			.extensions_mut()
			.remove::<Uncloneable<RoutingState>>()
			.expect("Uncloneable<RoutingState> should be inserted before routing started")
			.into_inner()
			.expect("RoutingState should always exist in Uncloneable");

		let resource_extensions = request
			.extensions_mut()
			.remove::<ResourceExtensions>()
			.expect("when layered, resource extensions must be inserted into the request");

		let mut args = Args {
			routing_state,
			resource_extensions,
			handler_extension: &(),
		};
		// Args::with_resource_extensions(resource_extensions);

		let response_future = self.handler.handle(request, &mut args);

		ResponseToResultFuture::from(response_future)
	}
}

// --------------------------------------------------
// ExtendedHandler

pub struct ExtendedHandler<H, E> {
	inner: H,
	extension: E,
}

impl<H, E> ExtendedHandler<H, E> {
	pub fn new(inner: H, extension: E) -> Self {
		Self { inner, extension }
	}
}

impl<H, B, E> Handler<B> for ExtendedHandler<H, E>
where
	H: Handler<B, E>,
	E: Clone + Send + Sync + 'static,
{
	type Response = H::Response;
	type Future = H::Future;

	#[inline]
	fn handle(&self, mut request: Request<B>, args: &mut Args) -> Self::Future {
		let routing_state = std::mem::take(&mut args.routing_state);
		let resource_extensions = args.resource_extensions.take();

		let mut args = Args {
			routing_state,
			resource_extensions,
			handler_extension: &self.extension,
		};

		self.inner.handle(request, &mut args)
	}
}

// ----------

#[derive(Clone)]
pub struct HandlerExtension<E>(E);

impl<E> FromRequestHead<E> for HandlerExtension<E>
where
	E: Clone + Sync,
{
	type Error = Infallible;

	#[inline]
	async fn from_request_head(
		head: &mut RequestHead,
		args: &mut Args<'_, E>,
	) -> Result<Self, Self::Error> {
		Ok(HandlerExtension(args.handler_extension.clone()))
	}
}

impl<B, E> FromRequest<B, E> for HandlerExtension<E>
where
	B: Send,
	E: Clone + Sync,
{
	type Error = Infallible;

	#[inline]
	async fn from_request(request: Request<B>, args: &mut Args<'_, E>) -> Result<Self, Self::Error> {
		Ok(HandlerExtension(args.handler_extension.clone()))
	}
}

// --------------------------------------------------------------------------------
// FinalHandler trait

trait FinalHandler
where
	Self: Handler<Response = Response, Future = BoxedFuture<Response>>,
{
	fn into_boxed_handler(self) -> BoxedHandler;
	fn boxed_clone(&self) -> BoxedHandler;
}

impl<H> FinalHandler for H
where
	H: Handler<Response = Response, Future = BoxedFuture<Response>> + Clone + Send + Sync + 'static,
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
		BoxedHandler(Box::new(handler))
	}
}

impl Default for BoxedHandler {
	#[inline(always)]
	fn default() -> Self {
		BoxedHandler(Box::new(DummyHandler::<BoxedFuture<Response>>::new()))
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
	type Future = BoxedFuture<Self::Response>;

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
	type Future = DefaultResponseFuture;

	#[inline(always)]
	fn handle(&self, _req: Request, _args: &mut Args) -> Self::Future {
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

impl Clone for DummyHandler<BoxedFuture<Response>> {
	fn clone(&self) -> Self {
		Self {
			_future_mark: PhantomData,
		}
	}
}

impl Handler for DummyHandler<BoxedFuture<Response>> {
	type Response = Response;
	type Future = BoxedFuture<Response>;

	#[inline(always)]
	fn handle(&self, _req: Request, _args: &mut Args) -> Self::Future {
		Box::pin(DefaultResponseFuture::new())
	}
}

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
	type Future = BoxedFuture<Response>;

	#[inline(always)]
	fn handle(&self, request: Request<B>, args: &mut Args<'_, ()>) -> Self::Future {
		self.0.handle(request.map(Body::new), args)
	}
}

impl From<BoxedHandler> for AdaptiveHandler {
	#[inline(always)]
	fn from(boxed_handler: BoxedHandler) -> Self {
		Self(boxed_handler)
	}
}

// --------------------------------------------------------------------------------
