use std::{
	any::Any,
	fmt::Debug,
	pin::Pin,
	task::{Context, Poll},
};

use hyper::HeaderMap;
use pin_project_lite::pin_project;

use super::utils::BoxedError;

// ----------

pub use http_body_util::{BodyExt, BodyStream, Empty, Full, StreamBody};
pub use hyper::body::{Body, Buf, Bytes, Frame, Incoming};

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

pub type BoxedBody = http_body_util::combinators::BoxBody<Bytes, BoxedError>;

// --------------------------------------------------

pin_project! {
	pub struct IncomingBody {
		#[pin] inner: InnerBody
	}
}

pin_project! {
	#[project = InnerBodyProjection]
	enum InnerBody {
		Incoming { #[pin] incoming: Incoming },
		Boxed { #[pin] boxed: BoxedBody },
	}
}

impl IncomingBody {
	fn from_incoming(incoming: Incoming) -> Self {
		Self {
			inner: InnerBody::Incoming { incoming },
		}
	}

	#[inline]
	pub fn new<B: Sized>(body: B) -> Self
	where
		B: Body + Send + Sync + 'static,
		B::Data: Debug,
		B::Error: Into<BoxedError>,
	{
		let mut some_body = Some(body);

		if let Some(some_incoming_body) =
			<dyn Any>::downcast_mut::<Option<IncomingBody>>(&mut some_body)
		{
			return some_incoming_body
				.take()
				.expect("Option should have been created from a valid value in a local scope");
		}

		let body =
			some_body.expect("Option should have been created from a valid value in a local scope");

		Self {
			inner: InnerBody::Boxed {
				boxed: BoxedBody::new(BodyAdapter::new(body)),
			},
		}
	}
}

impl Body for IncomingBody {
	type Data = Bytes;
	type Error = BoxedError;

	fn poll_frame(
		self: Pin<&mut Self>,
		cx: &mut Context<'_>,
	) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
		let self_projection = self.project();
		match self_projection.inner.project() {
			InnerBodyProjection::Incoming { incoming } => incoming.poll_frame(cx).map_err(Into::into),
			InnerBodyProjection::Boxed { boxed } => boxed.poll_frame(cx),
		}
	}
}

// -------------------------

pin_project! {
	struct BodyAdapter<B> { #[pin] inner: B }
}

impl<B> BodyAdapter<B> {
	fn new(inner: B) -> Self {
		Self { inner }
	}
}

impl<B> Body for BodyAdapter<B>
where
	B: Body,
	B::Data: Debug,
	B::Error: Into<BoxedError>,
{
	type Data = Bytes;
	type Error = BoxedError;

	fn poll_frame(
		self: Pin<&mut Self>,
		cx: &mut Context<'_>,
	) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
		self
			.project()
			.inner
			.poll_frame(cx)
			.map_ok(|frame| {
				if frame.is_data() {
					let bytes = if let Ok(mut data) = frame.into_data() {
						data.copy_to_bytes(data.remaining())
					} else {
						Bytes::new()
					};

					Frame::data(bytes)
				} else {
					let header_map = if let Ok(header_map) = frame.into_trailers() {
						header_map
					} else {
						HeaderMap::new()
					};

					Frame::trailers(header_map)
				}
			})
			.map_err(Into::into)
	}
}
