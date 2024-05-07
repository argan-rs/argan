use std::{
	convert::Infallible,
	future::Future,
	pin::{pin, Pin},
	task::{Context, Poll},
};

use argan_core::{
	body::{Body, Bytes, HttpBody},
	BoxedError,
};
use pin_project::pin_project;

use crate::response::{BoxedErrorResponse, Response};

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

#[pin_project]
pub struct ResultToResponseFuture<Fut>(#[pin] Fut);

impl<Fut> From<Fut> for ResultToResponseFuture<Fut> {
	fn from(inner: Fut) -> Self {
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
pub struct ResponseToResultFuture<Fut>(#[pin] Fut);

impl<Fut> From<Fut> for ResponseToResultFuture<Fut> {
	fn from(inner: Fut) -> Self {
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

#[pin_project]
pub struct ResponseBodyAdapterFuture<Fut>(#[pin] Fut);

impl<Fut, B, E> Future for ResponseBodyAdapterFuture<Fut>
where
	Fut: Future<Output = Result<Response<B>, E>>,
	B: HttpBody<Data = Bytes> + Send + Sync + 'static,
	B::Error: Into<BoxedError>,
	E: Into<BoxedErrorResponse>,
{
	type Output = Result<Response, BoxedErrorResponse>;

	fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
		match self.project().0.poll(cx) {
			Poll::Ready(result) => Poll::Ready(
				result
					.map(|response| {
						let (head_parts, body) = response.into_parts();
						let body = Body::new(body);

						Response::from_parts(head_parts, body)
					})
					.map_err(Into::into),
			),
			Poll::Pending => Poll::Pending,
		}
	}
}

impl<Fut> From<Fut> for ResponseBodyAdapterFuture<Fut> {
	fn from(inner: Fut) -> Self {
		Self(inner)
	}
}

// --------------------------------------------------------------------------------
