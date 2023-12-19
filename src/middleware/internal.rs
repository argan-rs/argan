use std::{
	fmt::Debug,
	future::Future,
	pin::Pin,
	task::{Context, Poll},
};

use pin_project::pin_project;

use crate::{
	body::{Body, IncomingBody},
	handler::Handler,
	request::Request,
	response::{IntoResponse, Response},
	utils::{BoxedError, BoxedFuture},
};

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

pub(crate) struct RequestBodyAdapter<H>(H);

impl<H, B> Handler<B> for RequestBodyAdapter<H>
where
	H: Handler<IncomingBody>,
	B: Body + Send + Sync + 'static,
	B::Data: Debug,
	B::Error: Into<BoxedError>,
{
	type Response = H::Response;
	type Future = H::Future;

	#[inline]
	fn handle(&self, request: Request<B>) -> Self::Future {
		let (parts, body) = request.into_parts();
		let body = IncomingBody::new(body);
		let request = Request::from_parts(parts, body);

		self.0.handle(request)
	}
}

impl<H> RequestBodyAdapter<H> {
	#[inline]
	pub(crate) fn wrap(handler: H) -> Self {
		Self(handler)
	}
}

// --------------------------------------------------------------------------------

pub(crate) struct IntoResponseAdapter<H>(H); // What a creative name!

impl<H, B> Handler<B> for IntoResponseAdapter<H>
where
	H: Handler<B>,
	H::Response: IntoResponse,
{
	type Response = Response;
	type Future = IntoResponseFuture<H::Future>;

	#[inline]
	fn handle(&self, request: Request<B>) -> Self::Future {
		let response_future = self.0.handle(request);

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

pub(crate) struct ResponseFutureBoxer<H>(H);

impl<H, B> Handler<B> for ResponseFutureBoxer<H>
where
	H: Handler<B, Response = Response>,
	H::Future: 'static,
{
	type Response = Response;
	type Future = BoxedFuture<Response>;

	fn handle(&self, request: Request<B>) -> Self::Future {
		let response_future = self.0.handle(request);

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
