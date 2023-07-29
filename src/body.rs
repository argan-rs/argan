use std::{
	any::{Any, TypeId},
	fmt::Debug,
	pin::Pin,
	task::{Context, Poll},
};

use hyper::HeaderMap;
use pin_project::pin_project;

// -------------------------

pub use hyper::body::{Body, Buf, Bytes, Frame, Incoming};

use super::utils::BoxedError;

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

pub type BoxedBody = http_body_util::combinators::BoxBody<Bytes, BoxedError>;

// --------------------------------------------------

#[pin_project]
pub struct IncomingBody(#[pin] InnerBody);

#[pin_project(project = InnerBodyProjection)]
enum InnerBody {
	Empty,
	Incoming(#[pin] Incoming),
	Boxed(#[pin] BoxedBody),
}

impl Default for IncomingBody {
	fn default() -> Self {
		Self(InnerBody::Empty)
	}
}

impl IncomingBody {
	fn from_incoming(body: Incoming) -> Self {
		Self(InnerBody::Incoming(body))
	}

	#[inline]
	pub fn new<B: Sized>(mut body: B) -> Self
	where
		B: Body + Send + Sync + 'static,
		B::Data: Debug,
		B::Error: Into<BoxedError>,
	{
		if body.type_id() == TypeId::of::<IncomingBody>() {
			let any_body = &mut body as &mut dyn Any;

			return std::mem::take(any_body.downcast_mut::<IncomingBody>().unwrap());
		}

		Self(InnerBody::Boxed(BoxedBody::new(BodyAdapter::new(body))))
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
		match self_projection.0.project() {
			InnerBodyProjection::Empty => Poll::Ready(None),
			InnerBodyProjection::Incoming(incoming) => {
				incoming.poll_frame(cx).map_err(|error| error.into())
			}
			InnerBodyProjection::Boxed(boxed) => boxed.poll_frame(cx),
		}
	}
}

// -------------------------

#[pin_project]
struct BodyAdapter<B>(#[pin] B);

impl<B> BodyAdapter<B> {
	fn new(body: B) -> Self {
		Self(body)
	}
}

impl<B> Body for BodyAdapter<B>
where
	B: Body + Send + 'static,
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
			.0
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
			.map_err(|error| error.into())
	}
}
