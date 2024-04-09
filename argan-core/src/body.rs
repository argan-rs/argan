//! Types and traits for request and response bodies.

// ----------

use std::{
	pin::{pin, Pin},
	task::{Context, Poll},
};

use http_body_util::BodyExt;

use crate::BoxedError;

// ----------

pub use bytes::*;
pub use http_body::Body as HttpBody;
pub use http_body::{Frame, SizeHint};

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

type BoxedBody = http_body_util::combinators::BoxBody<Bytes, BoxedError>;

// --------------------------------------------------

/// Body type used in requests and responses.
#[derive(Debug, Default)]
pub struct Body(BoxedBody);

impl Body {
	#[inline(always)]
	pub fn new<B: Sized>(body: B) -> Self
	where
		B: HttpBody<Data = Bytes> + Send + Sync + 'static,
		B::Error: Into<BoxedError>,
	{
		Self(BoxedBody::new(body.map_err(Into::into)))
	}
}

impl HttpBody for Body {
	type Data = Bytes;
	type Error = BoxedError;

	#[inline(always)]
	fn poll_frame(
		mut self: Pin<&mut Self>,
		cx: &mut Context<'_>,
	) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
		pin!(&mut self.0).poll_frame(cx)
	}
}

// --------------------------------------------------------------------------------
