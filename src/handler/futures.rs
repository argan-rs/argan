use std::{
	convert::Infallible,
	future::{Future, Ready},
	pin::{pin, Pin},
	task::{Context, Poll},
};

use pin_project::pin_project;

use crate::{common::BoxedFuture, response::Response};

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
	type Output = Response;

	fn poll(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Self::Output> {
		Poll::Ready(Response::default())
	}
}

// --------------------------------------------------------------------------------

pub(crate) enum ResponseFuture {
	Boxed(BoxedFuture<Response>),
	Ready(Ready<Response>),
}

impl From<BoxedFuture<Response>> for ResponseFuture {
	#[inline(always)]
	fn from(boxed_future: BoxedFuture<Response>) -> Self {
		Self::Boxed(boxed_future)
	}
}

impl From<Ready<Response>> for ResponseFuture {
	#[inline(always)]
	fn from(ready_future: Ready<Response>) -> Self {
		Self::Ready(ready_future)
	}
}

impl Future for ResponseFuture {
	type Output = Response;

	#[inline(always)]
	fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
		match *self {
			ResponseFuture::Boxed(ref mut boxed) => pin!(boxed).poll(cx),
			ResponseFuture::Ready(ref mut ready) => pin!(ready).poll(cx),
		}
	}
}

// --------------------------------------------------------------------------------
