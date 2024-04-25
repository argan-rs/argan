//! Traits and types for request handling.

// ----------

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

use argan_core::{
	body::{Body, Bytes, HttpBody},
	response::ResponseError,
	BoxedError, BoxedFuture,
};
use futures_util::FutureExt;
use http::{Extensions, Request, StatusCode};
use tower_service::Service as TowerService;

use crate::{
	common::{config::ConfigOption, BoxedAny, IntoArray, NodeExtensions, Uncloneable},
	middleware::Layer,
	request::{RequestContext, RequestHead},
	response::{BoxedErrorResponse, IntoResponse, Response},
	routing::RoutingState,
	StdError,
};

// --------------------------------------------------

pub(crate) mod futures;
use futures::{ResponseToResultFuture, ResultToResponseFuture};

mod impls;
pub(crate) use impls::*;

pub mod kind;
pub use kind::{
	_connect, _delete, _get, _head, _method, _mistargeted_request, _options, _patch, _post, _put,
	_trace, _wildcard_method,
};

use self::futures::ResponseBodyAdapterFuture;

pub(crate) mod request_handlers;

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

/// A trait for request handlers.
pub trait Handler<B = Body, Ext: Clone = ()> {
	type Response;
	type Error;
	type Future: Future<Output = Result<Self::Response, Self::Error>>;

	fn handle(&self, request: RequestContext<B>, args: Args<'_, Ext>) -> Self::Future;
}

// --------------------------------------------------
// IntoHandler

/// A trait for types that can be converted into a [`Handler`].
pub trait IntoHandler<Mark, B = Body, Ext: Clone = ()>: Sized {
	type Handler: Handler<B, Ext>;

	fn into_handler(self) -> Self::Handler;

	fn with_extension(self, handler_extension: Ext) -> ExtensionProviderHandler<Self::Handler, Ext> {
		ExtensionProviderHandler::new(self.into_handler(), handler_extension)
	}

	fn with_context<C, const N: usize>(
		self,
		context_elems: C,
	) -> ContextProviderHandler<Self::Handler>
	where
		C: IntoArray<HandlerContextElem, N>,
	{
		let context_elems = context_elems.into_array();

		let mut context = HandlerContext::default();

		for context_elem in context_elems {
			use HandlerContextElem::*;

			match context_elem {
				CookieKey(cookie_key) => {
					context.some_cookie_key = Some(cookie_key);
				}
			}
		}

		ContextProviderHandler::new(self.into_handler(), context)
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
// ExtensionHandler

/// An extension provider handler.
#[derive(Clone)]
pub struct ExtensionProviderHandler<H, Ext> {
	inner: H,
	extension: Ext,
}

impl<H, Ext> ExtensionProviderHandler<H, Ext> {
	pub(crate) fn new(inner: H, extension: Ext) -> Self {
		Self { inner, extension }
	}
}

impl<H, B, Ext> Handler<B> for ExtensionProviderHandler<H, Ext>
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
// ContextProviderHandler

/// A context provider handler.
pub struct ContextProviderHandler<H> {
	inner: H,
	context: HandlerContext,
}

impl<H> ContextProviderHandler<H> {
	fn new(handler: H, context: HandlerContext) -> Self {
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
		if let Some(cookie_key) = self.context.some_cookie_key.clone() {
			request = request.with_cookie_key(cookie_key);
		}

		self.inner.handle(request, args)
	}
}

// -------------------------
// HandlerContext

/// `Handler` context elements.
pub mod context {
	#[derive(Default)]
	pub(super) struct HandlerContext {
		pub some_cookie_key: Option<cookie::Key>,
	}

	option! {
		pub(super) HandlerContextElem {
			CookieKey(cookie::Key),
		}
	}

	/// Passes the cryptographic `Key` used for *private* and *signed* cookies
	/// as a `Handler` context.
	pub fn _cookie_key<K>(cookie_key: cookie::Key) -> HandlerContextElem {
		HandlerContextElem::CookieKey(cookie_key)
	}
}

use context::{HandlerContext, HandlerContextElem};

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
		Self(Box::new(DummyHandler))
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
		Self(Arc::new(DummyHandler))
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

// --------------------------------------------------
// DummyHandler

#[derive(Clone)]
pub(crate) struct DummyHandler;

impl Handler for DummyHandler {
	type Response = Response;
	type Error = BoxedErrorResponse;
	type Future = BoxedFuture<Result<Self::Response, Self::Error>>;

	fn handle(&self, request: RequestContext<Body>, args: Args<'_, ()>) -> Self::Future {
		Box::pin(async { Ok(Response::default()) })
	}
}

// --------------------------------------------------------------------------------
// TowerService

impl<S, B> Handler for S
where
	S: TowerService<Request<Body>, Response = Response<B>> + Clone,
	S::Error: Into<BoxedErrorResponse>,
	S::Future: Send + 'static,
	B: HttpBody<Data = Bytes> + Send + Sync + 'static,
	B::Error: Into<BoxedError>,
{
	type Response = Response;
	type Error = BoxedErrorResponse;
	type Future = BoxedFuture<Result<Response, BoxedErrorResponse>>;

	fn handle(&self, mut request: RequestContext, mut args: Args) -> Self::Future {
		let (mut request, routing_state, some_cookie_key) = request.into_parts();
		let args = args.to_owned();

		request
			.extensions_mut()
			.insert(Uncloneable::from((routing_state, some_cookie_key, args)));

		let future_response_result = self.clone().call(request);

		Box::pin(ResponseBodyAdapterFuture::from(future_response_result))
	}
}

// -------------------------
// HandlerService

/// `Handler` to `tower_service::Service` adapter.
#[derive(Clone)]
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
		let (routing_state, some_cookie_key, args) = request
			.extensions_mut()
			.remove::<Uncloneable<(RoutingState, Option<cookie::Key>, Args)>>()
			.expect(
				"request context data should be inserted in the Handler implementation for the Service",
			)
			.into_inner()
			.expect("Uncloneable must always have a valid value");

		let request = RequestContext::new(request, routing_state);

		self.handler.handle(request, args)
	}
}

// --------------------------------------------------
// Args

/// `Handler` arguments.
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

// --------------------------------------------------
// ErrorHandler

/// A trait for [ErrorResponse](crate::response::ErrorResponse) handlers.
pub trait ErrorHandler {
	fn handle_error(
		&mut self,
		error_response: BoxedErrorResponse,
	) -> impl Future<Output = Result<Response, BoxedErrorResponse>> + Send;
}

impl<Func, Fut> ErrorHandler for Func
where
	Func: FnMut(BoxedErrorResponse) -> Fut + Clone,
	Fut: Future<Output = Result<Response, BoxedErrorResponse>> + Send,
{
	fn handle_error(
		&mut self,
		error_response: BoxedErrorResponse,
	) -> impl Future<Output = Result<Response, BoxedErrorResponse>> + Send {
		self(error_response)
	}
}

// --------------------------------------------------------------------------------
