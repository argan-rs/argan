use super::body::*;
use super::utils::BoxedError;

// --------------------------------------------------

pub type Response<B = BoxedBody<Bytes, BoxedError>> = hyper::http::response::Response<B>;

// --------------------------------------------------

pub trait IntoResponse {
	fn into_response(self) -> Response;
}

// -------------------------

impl IntoResponse for Response {
	#[inline]
	fn into_response(self) -> Response {
		self
	}
}
