use std::{any::Any, convert::Infallible, fmt::Display};

use http::{
	header::{InvalidHeaderName, InvalidHeaderValue, LOCATION},
	response::Parts,
	HeaderMap, HeaderName, HeaderValue,
};

use crate::{
	body::{Body, BodyExt, Bytes, HttpBody},
	common::{BoxedError, SCOPE_VALIDITY},
	request::FromRequestHead,
};

// ----------

pub use http::StatusCode;

// --------------------------------------------------

mod error;
pub mod stream;

pub use error::{ErrorResponse, ResponseError};

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

pub type Response<B = Body> = http::response::Response<B>;
pub type ResponseHead = Parts;

pub type BoxedErrorResponse = Box<dyn ErrorResponse + 'static>;

// --------------------------------------------------
// IntoResponseHead trait

pub trait IntoResponseHead {
	fn into_response_head(self, head: ResponseHead) -> Result<ResponseHead, BoxedErrorResponse>;
}

// --------------------------------------------------
// IntoResponse trait

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

pub trait IntoResponseResult {
	fn into_response_result(self) -> Result<Response, BoxedErrorResponse>;
}

impl<R, E> IntoResponseResult for Result<R, E>
where
	R: IntoResponse,
	E: ErrorResponse,
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
// StatusCode

impl IntoResponse for StatusCode {
	#[inline]
	fn into_response(self) -> Response {
		let mut response = Response::default();
		*response.status_mut() = self;

		response
	}
}

// --------------------------------------------------
// Redirect

pub struct Redirect {
	status_code: StatusCode,
	uri: HeaderValue,
}

impl Redirect {
	pub fn permanently<U: AsRef<str>>(uri: U) -> Self {
		Self {
			status_code: StatusCode::PERMANENT_REDIRECT,
			uri: HeaderValue::from_str(uri.as_ref()).expect("uri must be a valid header value"),
		}
	}

	pub fn temporarily<U: AsRef<str>>(uri: U) -> Self {
		Self {
			status_code: StatusCode::TEMPORARY_REDIRECT,
			uri: HeaderValue::from_str(uri.as_ref()).expect("uri must be a valid header value"),
		}
	}

	pub fn to<U: AsRef<str>>(uri: U) -> Self {
		Self {
			status_code: StatusCode::SEE_OTHER,
			uri: HeaderValue::from_str(uri.as_ref()).expect("uri must be a valid header value"),
		}
	}
}

impl IntoResponse for Redirect {
	#[inline]
	fn into_response(self) -> Response {
		let mut response = Response::default();
		*response.status_mut() = self.status_code;
		response.headers_mut().insert(LOCATION, self.uri);

		response
	}
}

// --------------------------------------------------
// Infallible Error

impl IntoResponse for Infallible {
	#[inline]
	fn into_response(self) -> Response {
		Response::default()
	}
}

// --------------------------------------------------
// Unit ()

impl IntoResponse for () {
	#[inline]
	fn into_response(self) -> Response {
		Response::default()
	}
}

// --------------------------------------------------
// Option<T>

impl<T: IntoResponse> IntoResponse for Option<T> {
	#[inline]
	fn into_response(self) -> Response {
		match self {
			Some(value) => value.into_response(),
			None => {
				let mut response = Response::default();
				*response.status_mut() = StatusCode::NO_CONTENT;

				response
			}
		}
	}
}

// --------------------------------------------------
// Array of header (name, value) tuples

impl<N, V, const C: usize> IntoResponseHead for [(N, V); C]
where
	N: TryInto<HeaderName>,
	N::Error: crate::StdError + 'static,
	V: TryInto<HeaderValue>,
	V::Error: crate::StdError + 'static,
{
	fn into_response_head(self, mut head: ResponseHead) -> Result<ResponseHead, BoxedErrorResponse> {
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
	N::Error: crate::StdError + 'static,
	V: TryInto<HeaderValue>,
	V::Error: crate::StdError + 'static,
{
	fn into_response_result(self) -> Result<Response, BoxedErrorResponse> {
		let (head, body) = Response::default().into_parts();

		self
			.into_response_head(head)
			.map(|head| Response::from_parts(head, body))
	}
}

#[derive(Debug, crate::ImplError)]
pub enum HeaderError<NE, VE> {
	#[error("missing {0} header")]
	MissingHeader(HeaderName),
	#[error(transparent)]
	InvalidName(NE),
	#[error(transparent)]
	InvalidValue(VE),
}

impl<NE, VE> HeaderError<NE, VE> {
	pub(crate) fn from_missing_header(header_name: HeaderName) -> Self {
		Self::MissingHeader(header_name)
	}

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

macro_rules! impl_into_response_for_tuples {
	($t1:ident, $(($($t:ident),*),)? $tl:ident) => {
		#[allow(non_snake_case)]
		impl<$t1, $($($t,)*)? $tl> IntoResponseHead for ($t1, $($($t,)*)? $tl)
		where
			$t1: IntoResponseHead,
			$($($t: IntoResponseHead,)*)?
			$tl: IntoResponseHead,
		{
			fn into_response_head(self, mut head: ResponseHead) -> Result<ResponseHead, BoxedErrorResponse> {
				let ($t1, $($($t,)*)? $tl) = self;

				head = $t1.into_response_head(head)?;

				$($(head = $t.into_response_head(head)?;)*)?

				head = $tl.into_response_head(head)?;

				Ok(head)
			}
		}

		#[allow(non_snake_case)]
		impl<$($($t,)*)? $tl> IntoResponseResult for (StatusCode, $($($t,)*)? $tl)
		where
			$($($t: IntoResponseHead,)*)?
			$tl: IntoResponseResult,
		{
			fn into_response_result(self) -> Result<Response, BoxedErrorResponse> {
				let (status_code, $($($t,)*)? $tl) = self;

				let (head, body) = $tl.into_response_result()?.into_parts();

				$($(
					let head = $t.into_response_head(head)?;
				)*)?

				let mut response = Response::from_parts(head, body);
				*response.status_mut() = status_code;

				Ok(response)
			}
		}

		#[allow(non_snake_case)]
		impl<$t1, $($($t,)*)? $tl> IntoResponseResult for ($t1, $($($t,)*)? $tl)
		where
			$t1: IntoResponseHead,
			$($($t: IntoResponseHead,)*)?
			$tl: IntoResponseResult,
		{
			fn into_response_result(self) -> Result<Response, BoxedErrorResponse> {
				let ($t1, $($($t,)*)? $tl) = self;

				let (head, body) = $tl.into_response_result()?.into_parts();

				let head = $t1.into_response_head(head)?;

				$($(
					let head = $t.into_response_head(head)?;
				)*)?

				Ok(Response::from_parts(head, body))
			}
		}
	};
}

call_for_tuples!(impl_into_response_for_tuples!);

// --------------------------------------------------------------------------------
