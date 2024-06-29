//! Response types and conversion traits into them.

// ----------

use http::{HeaderName, HeaderValue};

use crate::body::Body;

// ----------

pub use http::response::Builder;

// --------------------------------------------------

mod error;
pub use error::{ErrorResponse, ResponseError};

mod impls;

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

pub type Response<B = Body> = http::response::Response<B>;
pub type ResponseHeadParts = http::response::Parts;

pub type BoxedErrorResponse = Box<dyn ErrorResponse + Send + Sync>;
pub type ResponseResult = Result<Response, BoxedErrorResponse>;

// --------------------------------------------------------------------------------

// --------------------------------------------------
// IntoResponseHeadParts trait

/// Implemented by types that form or can be converted into a type that forms the
/// [`ResponseHeadParts`].
pub trait IntoResponseHeadParts {
	fn into_response_head(
		self,
		head: ResponseHeadParts,
	) -> Result<ResponseHeadParts, BoxedErrorResponse>;
}

// --------------------------------------------------
// IntoResponse trait

/// Implemented by types that can be converted into the [`Response`] type.
pub trait IntoResponse {
	fn into_response(self) -> Response;
}

impl IntoResponse for Response {
	fn into_response(self) -> Response {
		self
	}
}

// --------------------------------------------------
// IntoResponseResult trait

/// Implemented by types that can be converted into the [`ResponseResult`] type.
pub trait IntoResponseResult {
	fn into_response_result(self) -> Result<Response, BoxedErrorResponse>;
}

impl<R, E> IntoResponseResult for Result<R, E>
where
	R: IntoResponseResult,
	E: Into<BoxedErrorResponse>,
{
	fn into_response_result(self) -> Result<Response, BoxedErrorResponse> {
		self
			.map_err(Into::into)
			.and_then(IntoResponseResult::into_response_result)
	}
}

impl<R> IntoResponseResult for R
where
	R: IntoResponse,
{
	fn into_response_result(self) -> Result<Response, BoxedErrorResponse> {
		Ok(self.into_response())
	}
}

// --------------------------------------------------------------------------------
