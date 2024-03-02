use std::{
	any::Any,
	pin::{pin, Pin},
	task::{Context, Poll},
};

use hyper::HeaderMap;
use pin_project::pin_project;

use super::common::{BoxedError, SCOPE_VALIDITY};

// ----------

pub use http_body_util::BodyExt;
pub use hyper::body::{Body as HttpBody, Buf, Bytes, Frame, Incoming};

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

type BoxedBody = http_body_util::combinators::BoxBody<Bytes, BoxedError>;

// --------------------------------------------------

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
