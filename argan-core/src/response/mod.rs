//! Response types and conversion traits into them.

// ----------

use http::{HeaderName, HeaderValue, StatusCode};

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

// --------------------------------------------------------------------------------

// --------------------------------------------------
// IntoResponseHead trait

/// Implemented by types that form or can be converted into a type that forms the
/// [ResponseHeadParts].
pub trait IntoResponseHeadParts {
	fn into_response_head(
		self,
		head: ResponseHeadParts,
	) -> Result<ResponseHeadParts, BoxedErrorResponse>;
}

// --------------------------------------------------
// IntoResponse trait

/// Implemented by types that can be converted into the [Response] type.
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

/// Implemented by types or error types that can be converted into the [Response] type.
pub trait IntoResponseResult {
	fn into_response_result(self) -> Result<Response, BoxedErrorResponse>;
}

impl<R, E> IntoResponseResult for Result<R, E>
where
	R: IntoResponse,
	E: Into<BoxedErrorResponse>,
{
	fn into_response_result(self) -> Result<Response, BoxedErrorResponse> {
		self.map(IntoResponse::into_response).map_err(Into::into)
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

// --------------------------------------------------
// Array of header (name, value) tuples

impl<N, V, const C: usize> IntoResponseHeadParts for [(N, V); C]
where
	N: TryInto<HeaderName>,
	N::Error: crate::StdError + Send + Sync + 'static,
	V: TryInto<HeaderValue>,
	V::Error: crate::StdError + Send + Sync + 'static,
{
	fn into_response_head(
		self,
		mut head: ResponseHeadParts,
	) -> Result<ResponseHeadParts, BoxedErrorResponse> {
		for (key, value) in self {
			let header_name = TryInto::<HeaderName>::try_into(key)
				.map_err(HeaderError::<N::Error, V::Error>::from_name_error)?;

			let header_value = TryInto::<HeaderValue>::try_into(value)
				.map_err(HeaderError::<N::Error, V::Error>::from_value_error)?;

			head.headers.insert(header_name, header_value);
		}

		Ok(head)
	}
}

impl<N, V, const C: usize> IntoResponseResult for [(N, V); C]
where
	N: TryInto<HeaderName>,
	N::Error: crate::StdError + Send + Sync + 'static,
	V: TryInto<HeaderValue>,
	V::Error: crate::StdError + Send + Sync + 'static,
{
	fn into_response_result(self) -> Result<Response, BoxedErrorResponse> {
		let (head, body) = Response::default().into_parts();

		self
			.into_response_head(head)
			.map(|head| Response::from_parts(head, body))
	}
}

#[derive(Debug, crate::ImplError)]
enum HeaderError<NE, VE> {
	#[error(transparent)]
	InvalidName(NE),
	#[error(transparent)]
	InvalidValue(VE),
}

impl<NE, VE> HeaderError<NE, VE> {
	pub(crate) fn from_name_error(name_error: NE) -> Self {
		Self::InvalidName(name_error)
	}

	pub(crate) fn from_value_error(value_error: VE) -> Self {
		Self::InvalidValue(value_error)
	}
}

impl<NE, VE> IntoResponse for HeaderError<NE, VE> {
	fn into_response(self) -> Response {
		StatusCode::INTERNAL_SERVER_ERROR.into_response()
	}
}

// --------------------------------------------------------------------------------
