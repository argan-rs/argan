use super::body::*;

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

pub type Response = hyper::http::response::Response<BoxedBody>;

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
