use std::{
	convert::Infallible,
	future::Future,
	pin::Pin,
	task::{Context, Poll},
};

use pin_project_lite::pin_project;

use crate::response::Response;

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

pin_project! {
	pub struct ResultToResponseFuture<F> {
		#[pin] inner: F
	}
}

impl<F> From<F> for ResultToResponseFuture<F> {
	fn from(inner: F) -> Self {
		Self { inner }
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
			.inner
			.poll(cx)
			.map(|output| output.expect("Err should be infallible"))
	}
}

// --------------------------------------------------------------------------------

pin_project! {
	pub struct ResponseToResultFuture<F> {
		#[pin] inner: F
	}
}

impl<F> From<F> for ResponseToResultFuture<F> {
	fn from(inner: F) -> Self {
		Self { inner }
	}
}

impl<F, R> Future for ResponseToResultFuture<F>
where
	F: Future<Output = R>,
{
	type Output = Result<R, Infallible>;

	#[inline]
	fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
		self.project().inner.poll(cx).map(|output| Ok(output))
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
	type Output = Response;

	fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
		Poll::Ready(Response::default())
	}
}

// --------------------------------------------------------------------------------
