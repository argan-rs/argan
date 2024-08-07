//! Traits and types for request handling.

// ----------

use std::{
	borrow::Cow,
	future::Future,
	sync::Arc,
	task::{Context, Poll},
};

#[cfg(feature = "peer-addr")]
use std::net::SocketAddr;

use argan_core::{
	body::{Body, Bytes, HttpBody},
	BoxedError, BoxedFuture,
};
use http::Request;
use tower_service::Service as TowerService;

use crate::{
	common::{IntoArray, NodeExtension, Uncloneable},
	middleware::Layer,
	request::{routing::RoutingState, RequestContext, RequestContextProperties},
	response::{BoxedErrorResponse, Response},
};

// --------------------------------------------------

pub(crate) mod futures;

mod impls;
pub(crate) use impls::*;

pub(crate) mod kind;

#[doc(inline)]
pub use kind::HandlerSetter;

use self::futures::ResponseBodyAdapterFuture;

pub(crate) mod request_handlers;

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

/// A trait for request handlers.
pub trait Handler<B = Body, Ext: Clone = ()> {
	type Response;
	type Error;
	type Future: Future<Output = Result<Self::Response, Self::Error>>;

	fn handle(&self, request_context: RequestContext<B>, args: Args<'_, Ext>) -> Self::Future;
}

// --------------------------------------------------
// IntoHandler

/// A trait for types that can be converted into a [`Handler`].
pub trait IntoHandler<Mark, B = Body, Ext: Clone = ()>: Sized {
	type Handler: Handler<B, Ext>;

	fn into_handler(self) -> Self::Handler;

	/// Provides the handler with a user-defined extension.
	fn with_extension(self, handler_extension: Ext) -> ExtensionProviderHandler<Self::Handler, Ext> {
		ExtensionProviderHandler::new(self.into_handler(), handler_extension)
	}

	/// Provides the handler with pre-defined properties.
	fn with_property<P, const N: usize>(self, properties: P) -> PropertyProviderHandler<Self::Handler>
	where
		P: IntoArray<HandlerProperty, N>,
	{
		#![allow(unused)]

		let properties = properties.into_array();

		let mut context_properties = RequestContextProperties::default();

		#[cfg(any(feature = "private-cookies", feature = "signed-cookies"))]
		for context_elem in properties {
			use HandlerProperty::*;

			match context_elem {
				CookieKey(cookie_key) => {
					context_properties.set_cookie_key(cookie_key);
				}
			}
		}

		PropertyProviderHandler::new(self.into_handler(), context_properties)
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
	fn handle(&self, request_context: RequestContext<B>, args: Args) -> Self::Future {
		let args = Args {
			node_extension: args.node_extension,
			handler_extension: Cow::Borrowed(&self.extension),
		};

		self.inner.handle(request_context, args)
	}
}

// --------------------------------------------------
// ContextProviderHandler

/// A context provider handler.
#[derive(Clone)]
pub struct PropertyProviderHandler<H> {
	inner: H,
	context_properties: RequestContextProperties,
}

impl<H> PropertyProviderHandler<H> {
	fn new(handler: H, context_properties: RequestContextProperties) -> Self {
		Self {
			inner: handler,
			context_properties,
		}
	}
}

impl<H, B, Ext> Handler<B, Ext> for PropertyProviderHandler<H>
where
	H: Handler<B, Ext>,
	Ext: Clone,
{
	type Response = H::Response;
	type Error = H::Error;
	type Future = H::Future;

	#[inline]
	fn handle(&self, mut request_context: RequestContext<B>, args: Args<'_, Ext>) -> Self::Future {
		request_context.clone_valid_properties_from(&self.context_properties);

		self.inner.handle(request_context, args)
	}
}

// -------------------------
// HandlerContext

/// `Handler` properties.
///
/// ```
/// use argan::{
///   data::cookies::Key,
///   handler::{HandlerCookieKey, HandlerSetter, IntoHandler},
///   http::Method,
///   Resource,
/// };
///
/// let mut resource = Resource::new("/resource");
/// resource.set_handler_for(
///   Method::GET.to(
///     (|| async { /* ... */ }).with_property(HandlerCookieKey.set_to(Key::generate()))
///   ),
/// );
/// ```
pub mod properties {
	option! {
		pub(super) HandlerProperty {
			#[cfg(any(feature = "private-cookies", feature = "signed-cookies"))]
			CookieKey(cookie::Key),
		}
	}

	/// A type that represents the *cookie key* as a property.
	#[cfg(any(feature = "private-cookies", feature = "signed-cookies"))]
	pub struct HandlerCookieKey;

	/// Passes the cryptographic `Key` used for *private* and *signed* cookies
	/// as a handler property.
	#[cfg(any(feature = "private-cookies", feature = "signed-cookies"))]
	impl HandlerCookieKey {
		pub fn set_to<K: Into<cookie::Key>>(self, key: K) -> HandlerProperty {
			HandlerProperty::CookieKey(key.into())
		}
	}
}

#[cfg(any(feature = "private-cookies", feature = "signed-cookies"))]
pub use properties::HandlerCookieKey;
use properties::HandlerProperty;

// --------------------------------------------------------------------------------
// Boxable handler

/// Boxable handlers that directly return [`Response`] or [`BoxedErrorResponse`]
/// without any conversion.
pub trait BoxableHandler
where
	Self: Handler<
		Response = Response,
		Error = BoxedErrorResponse,
		Future = BoxedFuture<Result<Response, BoxedErrorResponse>>,
	>,
{
	// ...
}

impl<H> BoxableHandler for H
where
	H: Handler<
		Response = Response,
		Error = BoxedErrorResponse,
		Future = BoxedFuture<Result<Response, BoxedErrorResponse>>,
	>,
{
	// ...
}

// --------------------------------------------------------------------------------
// FinalHandler trait

trait FinalHandler
where
	Self: BoxableHandler,
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
	fn handle(&self, request_context: RequestContext, args: Args) -> Self::Future {
		self.0.handle(request_context, args)
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
	fn handle(&self, request_context: RequestContext, args: Args) -> Self::Future {
		self.0.handle(request_context, args)
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

	fn handle(&self, _request_context: RequestContext<Body>, _args: Args<'_, ()>) -> Self::Future {
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

	fn handle(&self, request_context: RequestContext, args: Args) -> Self::Future {
		#[cfg(not(feature = "peer-addr"))]
		let (mut request, routing_state, context_properties) = request_context.into_parts();

		#[cfg(feature = "peer-addr")]
		let (peer_addr, mut request, routing_state, context_properties) = request_context.into_parts();

		let args = args.into_owned();

		#[cfg(not(feature = "peer-addr"))]
		request
			.extensions_mut()
			.insert(Uncloneable::from((routing_state, context_properties, args)));

		#[cfg(feature = "peer-addr")]
		request.extensions_mut().insert(Uncloneable::from((
			peer_addr,
			routing_state,
			context_properties,
			args,
		)));

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

	fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
		Poll::Ready(Ok(()))
	}

	#[inline]
	fn call(&mut self, mut request: Request<B>) -> Self::Future {
		#[cfg(not(feature = "peer-addr"))]
		let (routing_state, properties, args) = request
			.extensions_mut()
			.remove::<Uncloneable<(RoutingState, RequestContextProperties, Args)>>()
			.expect(
				"request context data should be inserted in the Handler implementation for the Service",
			)
			.into_inner()
			.expect("Uncloneable must always have a valid value");

		#[cfg(feature = "peer-addr")]
		let (peer_addr, routing_state, properties, args) = request
			.extensions_mut()
			.remove::<Uncloneable<(SocketAddr, RoutingState, RequestContextProperties, Args)>>()
			.expect(
				"request context data should be inserted in the Handler implementation for the Service",
			)
			.into_inner()
			.expect("Uncloneable must always have a valid value");

		let request_context = RequestContext::new(
			#[cfg(feature = "peer-addr")]
			peer_addr,
			request,
			routing_state,
			properties,
		);

		self.handler.handle(request_context, args)
	}
}

// --------------------------------------------------
// Args

/// `Handler` arguments.
#[non_exhaustive]
pub struct Args<'e, HandlerExt: Clone = ()> {
	pub node_extension: Cow<'e, NodeExtension>,
	pub handler_extension: Cow<'e, HandlerExt>,
}

impl Args<'_, ()> {
	pub(crate) fn new() -> Args<'static, ()> {
		Args {
			node_extension: Cow::Owned(NodeExtension::new()),
			handler_extension: Cow::Borrowed(&()),
		}
	}

	#[inline(always)]
	pub(crate) fn new_with_node_extension_ref(node_extension: &NodeExtension) -> Args<'_, ()> {
		Args {
			node_extension: Cow::Borrowed(node_extension),
			handler_extension: Cow::Borrowed(&()),
		}
	}
}

impl<HandlerExt: Clone> Args<'_, HandlerExt> {
	#[inline(always)]
	pub(crate) fn into_owned(self) -> Args<'static, HandlerExt> {
		Args {
			node_extension: Cow::Owned(self.node_extension.into_owned()),
			handler_extension: Cow::Owned(self.handler_extension.into_owned()),
		}
	}
}

// --------------------------------------------------------------------------------
