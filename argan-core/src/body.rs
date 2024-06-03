//! Types and traits for request and response bodies.

// ----------

use std::{
	borrow::Cow,
	pin::{pin, Pin},
	task::{Context, Poll},
};

use http_body_util::{BodyExt, Empty, Full};

use crate::BoxedError;

// ----------

pub use bytes::*;
pub use http_body::Body as HttpBody;
pub use http_body::{Frame, SizeHint};

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

type BoxedBody = http_body_util::combinators::BoxBody<Bytes, BoxedError>;

// --------------------------------------------------
// Body

/// An [`HttpBody`] type used in requests and responses.
#[derive(Debug, Default)]
pub struct Body(BoxedBody);

impl Body {
	#[inline(always)]
	pub fn new<B>(body: B) -> Self
	where
		B: HttpBody<Data = Bytes> + Sized + Send + Sync + 'static,
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

// -------------------------

impl From<()> for Body {
	#[inline]
	fn from(_value: ()) -> Self {
		Self::default()
	}
}

impl From<Bytes> for Body {
	#[inline]
	fn from(bytes: Bytes) -> Self {
		Self::new(Full::from(bytes))
	}
}

impl From<Empty<Bytes>> for Body {
	#[inline]
	fn from(empty_body: Empty<Bytes>) -> Self {
		Self::new(empty_body)
	}
}

impl From<Full<Bytes>> for Body {
	#[inline]
	fn from(full_body: Full<Bytes>) -> Self {
		Self::new(full_body)
	}
}

impl From<&'static str> for Body {
	#[inline]
	fn from(str_body: &'static str) -> Self {
		Cow::<'_, str>::Borrowed(str_body).into()
	}
}

impl From<String> for Body {
	#[inline]
	fn from(string_body: String) -> Self {
		Cow::<'_, str>::Owned(string_body).into()
	}
}

impl From<Cow<'static, str>> for Body {
	#[inline]
	fn from(cow_body: Cow<'static, str>) -> Self {
		Self::new(Full::from(cow_body))
	}
}

impl From<&'static [u8]> for Body {
	#[inline]
	fn from(slice_body: &'static [u8]) -> Self {
		Cow::<'_, [u8]>::Borrowed(slice_body).into()
	}
}

impl From<Vec<u8>> for Body {
	#[inline]
	fn from(vec_body: Vec<u8>) -> Self {
		Cow::<'_, [u8]>::Owned(vec_body).into()
	}
}

impl From<Cow<'static, [u8]>> for Body {
	#[inline]
	fn from(cow_body: Cow<'static, [u8]>) -> Self {
		Body::new(Full::from(cow_body))
	}
}

// --------------------------------------------------------------------------------
