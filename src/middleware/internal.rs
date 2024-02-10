use std::{
	fmt::Debug,
	future::Future,
	pin::Pin,
	task::{Context, Poll},
};

use bytes::Bytes;
use http::Extensions;
use http_body_util::BodyExt;
use pin_project::pin_project;

use crate::{
	body::{Body, HttpBody},
	common::{BoxedError, BoxedFuture},
	handler::{AdaptiveHandler, Args, BoxedHandler, DummyHandler, Handler},
	request::Request,
	response::{IntoResponse, Response},
};

use super::Layer;

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

// #[derive(Clone)]
// pub(crate) struct RequestBodyAdapter<H>(H);
//
// impl<H, B> Handler<B> for RequestBodyAdapter<H>
// where
// 	H: Handler,
// 	B: HttpBody<Data = Bytes> + Send + Sync + 'static,
// 	B::Error: Into<BoxedError>,
// {
// 	type Response = H::Response;
// 	type Future = H::Future;
//
// 	#[inline]
// 	fn handle(&self, request: Request<B>, args: &mut Args) -> Self::Future {
// 		let request = request.map(Body::new);
//
// 		self.0.handle(request, args)
// 	}
// }
//
// impl<H> RequestBodyAdapter<H> {
// 	#[inline]
// 	pub(crate) fn wrap(handler: H) -> Self {
// 		Self(handler)
// 	}
// }

// --------------------------------------------------------------------------------

#[derive(Clone)]
pub(crate) struct IntoResponseAdapter<H>(H);

impl<H, B> Handler<B> for IntoResponseAdapter<H>
where
	H: Handler<B>,
	H::Response: IntoResponse,
{
	type Response = Response;
	type Future = IntoResponseFuture<H::Future>;

	#[inline]
	fn handle(&self, request: Request<B>, args: &mut Args) -> Self::Future {
		let response_future = self.0.handle(request, args);

		IntoResponseFuture::from(response_future)
	}
}

impl<H> IntoResponseAdapter<H> {
	#[inline]
	pub(crate) fn wrap(handler: H) -> Self {
		Self(handler)
	}
}

// ----------

#[pin_project]
pub(crate) struct IntoResponseFuture<F>(#[pin] F);

impl<F> From<F> for IntoResponseFuture<F> {
	fn from(inner: F) -> Self {
		Self(inner)
	}
}

impl<F, R> Future for IntoResponseFuture<F>
where
	F: Future<Output = R>,
	R: IntoResponse,
{
	type Output = Response;

	#[inline]
	fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
		self
			.project()
			.0
			.poll(cx)
			.map(|output| output.into_response())
	}
}

// --------------------------------------------------------------------------------

#[derive(Clone)]
pub(crate) struct ResponseFutureBoxer<H>(H);

impl<H, B> Handler<B> for ResponseFutureBoxer<H>
where
	H: Handler<B, Response = Response>,
	H::Future: 'static,
{
	type Response = Response;
	type Future = BoxedFuture<Response>;

	fn handle(&self, request: Request<B>, args: &mut Args) -> Self::Future {
		let response_future = self.0.handle(request, args);

		Box::pin(response_future)
	}
}

impl<H> ResponseFutureBoxer<H> {
	#[inline]
	pub(crate) fn wrap(handler: H) -> Self {
		Self(handler)
	}
}

// --------------------------------------------------------------------------------

#[derive(Clone)]
pub(crate) struct RequestExtensionsModifierLayer(BoxedExtensionsModifier);

impl RequestExtensionsModifierLayer {
	pub(crate) fn new<Func>(modifier: Func) -> Self
	where
		Func: Fn(&mut Extensions) + Clone + Send + Sync + 'static,
	{
		Self(BoxedExtensionsModifier::new(modifier))
	}
}

impl Layer<AdaptiveHandler> for RequestExtensionsModifierLayer {
	type Handler = RequestExtensionsModifier<AdaptiveHandler>;

	fn wrap(&self, handler: AdaptiveHandler) -> Self::Handler {
		RequestExtensionsModifier::new(handler, self.0.clone())
	}
}

// ----------

#[derive(Clone)]
pub(crate) struct RequestExtensionsModifier<H> {
	inner_handler: H,
	boxed_modifier: BoxedExtensionsModifier,
}

impl<H> RequestExtensionsModifier<H> {
	fn new(handler: H, boxed_modifier: BoxedExtensionsModifier) -> Self {
		Self {
			inner_handler: handler,
			boxed_modifier,
		}
	}
}

impl<H, B> Handler<B> for RequestExtensionsModifier<H>
where
	H: Handler<B>,
{
	type Response = H::Response;
	type Future = H::Future;

	#[inline(always)]
	fn handle(&self, mut request: Request<B>, args: &mut Args<'_, ()>) -> Self::Future {
		self.boxed_modifier.0(request.extensions_mut());

		self.inner_handler.handle(request, args)
	}
}

// -------------------------

trait ExtensionsModifier: Fn(&mut Extensions) {
	fn boxed_clone(&self) -> BoxedExtensionsModifier;
}

impl<Func> ExtensionsModifier for Func
where
	Func: Fn(&mut Extensions) + Clone + Send + Sync + 'static,
{
	fn boxed_clone(&self) -> BoxedExtensionsModifier {
		BoxedExtensionsModifier::new(self.clone())
	}
}

// -------------------------

struct BoxedExtensionsModifier(Box<dyn ExtensionsModifier + Send + Sync + 'static>);

impl BoxedExtensionsModifier {
	pub(crate) fn new<Func>(modifier: Func) -> Self
	where
		Func: Fn(&mut Extensions) + Clone + Send + Sync + 'static,
	{
		Self(Box::new(modifier))
	}
}

impl Clone for BoxedExtensionsModifier {
	fn clone(&self) -> Self {
		self.0.boxed_clone()
	}
}

// --------------------------------------------------------------------------------
