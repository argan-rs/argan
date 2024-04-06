use std::{
	convert::Infallible,
	future::{Future, Ready},
	pin::{pin, Pin},
	task::{Context, Poll},
};

use argan_core::response::IntoResponse;
use futures_util::FutureExt;
use pin_project::pin_project;

use crate::response::{BoxedErrorResponse, Response};

use super::ErrorHandler;

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

#[pin_project]
pub struct ResultToResponseFuture<F>(#[pin] F);

impl<F> From<F> for ResultToResponseFuture<F> {
	fn from(inner: F) -> Self {
		Self(inner)
	}
}

impl<F, R> Future for ResultToResponseFuture<F>
where
	F: Future<Output = Result<R, Infallible>>,
{
	type Output = R;

	#[inline]
	fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
		self
			.project()
			.0
			.poll(cx)
			.map(|output| output.expect("Err should be infallible"))
	}
}

// --------------------------------------------------------------------------------

#[pin_project]
pub struct ResponseToResultFuture<F>(#[pin] F);

impl<F> From<F> for ResponseToResultFuture<F> {
	fn from(inner: F) -> Self {
		Self(inner)
	}
}

impl<F, R> Future for ResponseToResultFuture<F>
where
	F: Future<Output = R>,
{
	type Output = Result<R, Infallible>;

	#[inline]
	fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
		self.project().0.poll(cx).map(|output| Ok(output))
	}
}

// --------------------------------------------------------------------------------

pub(crate) struct DefaultResponseFuture;

impl DefaultResponseFuture {
	pub(crate) fn new() -> Self {
		Self
	}
}

impl Future for DefaultResponseFuture {
	type Output = Result<Response, BoxedErrorResponse>;

	fn poll(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Self::Output> {
		Poll::Ready(Ok(Response::default()))
	}
}

// --------------------------------------------------------------------------------

#[pin_project]
pub struct ResponseResultHandlerFuture<Fut, ErrH> {
	#[pin]
	inner: Fut,
	error_handler: ErrH,
}

impl<Fut, ErrH> ResponseResultHandlerFuture<Fut, ErrH> {
	pub(crate) fn new(inner: Fut, error_handler: ErrH) -> Self {
		Self {
			inner,
			error_handler,
		}
	}
}

impl<Fut, R, E, ErrH> Future for ResponseResultHandlerFuture<Fut, ErrH>
where
	Fut: Future<Output = Result<R, E>>,
	R: IntoResponse,
	E: Into<BoxedErrorResponse>,
	ErrH: ErrorHandler<E>,
{
	type Output = Result<Response, E>;

	fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
		let self_projection = self.project();

		match self_projection.inner.poll(cx) {
			Poll::Ready(result) => match result {
				Ok(response) => Poll::Ready(Ok(response.into_response())),
				Err(error) => pin!(self_projection.error_handler.handle_error(error)).poll(cx),
			},
			Poll::Pending => Poll::Pending,
		}
	}
}
