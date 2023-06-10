use super::body::*;
use super::utils::BoxedError;

// --------------------------------------------------

pub type Response<B: Body = BoxedBody<Bytes, BoxedError>> = hyper::Response<B>;

// --------------------------------------------------

pub trait IntoResponse<B>
where
	B: Body,
{
	fn into_response(self) -> Response<B>;
}

// -------------------------

impl<B> IntoResponse<B> for Response<B>
where
	B: Body,
{
	#[inline]
	fn into_response(self) -> Response<B> {
		self
	}
}
