use std::{
	fmt::Debug,
	future::Future,
	pin::Pin,
	task::{Context, Poll},
};

use bytes::Bytes;
use http_body_util::BodyExt;
use pin_project::pin_project;

use crate::{
	body::{Body, HttpBody},
	common::{BoxedError, BoxedFuture},
	handler::{BoxedHandler, DummyHandler, FinalHandler, Handler},
	request::Request,
	resource::ResourceExtensions,
	response::{IntoResponse, Response},
};

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

#[derive(Clone)]
pub(crate) struct RequestBodyAdapter<H>(H);

impl<H, B> Handler<B> for RequestBodyAdapter<H>
where
	H: Handler,
	B: HttpBody<Data = Bytes> + Send + Sync + 'static,
	B::Error: Into<BoxedError>,
{
	type Response = H::Response;
	type Future = H::Future;

	#[inline]
	fn handle(&self, request: Request<B>, resource_extensions: ResourceExtensions) -> Self::Future {
		let (parts, body) = request.into_parts();
		let body = Body::new(body);
		let request = Request::from_parts(parts, body);

		self.0.handle(request, resource_extensions)
	}
}

impl<H> RequestBodyAdapter<H> {
	#[inline]
	pub(crate) fn wrap(handler: H) -> Self {
		Self(handler)
	}
}

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
	fn handle(&self, request: Request<B>, resource_extensions: ResourceExtensions) -> Self::Future {
		let response_future = self.0.handle(request, resource_extensions);

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

	fn handle(&self, request: Request<B>, resource_extensions: ResourceExtensions) -> Self::Future {
		let response_future = self.0.handle(request, resource_extensions);

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
