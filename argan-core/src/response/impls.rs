use std::{borrow::Cow, convert::Infallible};

use bytes::Bytes;
use http::{header, HeaderMap, StatusCode};
use http_body_util::{Empty, Full};

use super::*;

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

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
// HeaderMap

impl IntoResponseHeadParts for HeaderMap {
	#[inline]
	fn into_response_head(
		self,
		mut head: ResponseHeadParts,
	) -> Result<ResponseHeadParts, BoxedErrorResponse> {
		head.headers.extend(self);

		Ok(head)
	}
}

impl IntoResponse for HeaderMap {
	fn into_response(self) -> Response {
		let mut response = ().into_response();
		*response.headers_mut() = self;

		response
	}
}

// --------------------------------------------------
// Bytes

impl IntoResponse for Bytes {
	#[inline]
	fn into_response(self) -> Response {
		let mut response = Full::from(self).into_response();
		response.headers_mut().insert(
			header::CONTENT_TYPE,
			HeaderValue::from_static(mime::APPLICATION_OCTET_STREAM.as_ref()),
		);

		response
	}
}

// --------------------------------------------------
// Empty<Bytes>

impl IntoResponse for Empty<Bytes> {
	#[inline]
	fn into_response(self) -> Response {
		Response::new(Body::new(self))
	}
}

// --------------------------------------------------
// Full<Bytes>

impl IntoResponse for Full<Bytes> {
	#[inline]
	fn into_response(self) -> Response {
		Response::new(Body::new(self))
	}
}

// --------------------------------------------------
// &'static str

impl IntoResponse for &'static str {
	#[inline]
	fn into_response(self) -> Response {
		Cow::<'_, str>::Borrowed(self).into_response()
	}
}

// --------------------------------------------------
// String

impl IntoResponse for String {
	#[inline]
	fn into_response(self) -> Response {
		Cow::<'_, str>::Owned(self).into_response()
	}
}

// --------------------------------------------------
// Cow<'static, str>

impl IntoResponse for Cow<'static, str> {
	#[inline]
	fn into_response(self) -> Response {
		let mut response = Full::from(self).into_response();
		response.headers_mut().insert(
			header::CONTENT_TYPE,
			HeaderValue::from_static(mime::TEXT_PLAIN_UTF_8.as_ref()),
		);

		response
	}
}

// --------------------------------------------------
// &'static [u8]

impl IntoResponse for &'static [u8] {
	#[inline]
	fn into_response(self) -> Response {
		Cow::<'_, [u8]>::Borrowed(self).into_response()
	}
}

// --------------------------------------------------
// Vec<u8>

impl IntoResponse for Vec<u8> {
	#[inline]
	fn into_response(self) -> Response {
		Cow::<'_, [u8]>::Owned(self).into_response()
	}
}

// --------------------------------------------------
// Cow<'static, [u8]>

impl IntoResponse for Cow<'static, [u8]> {
	#[inline]
	fn into_response(self) -> Response {
		let mut response = Full::from(self).into_response();
		response.headers_mut().insert(
			header::CONTENT_TYPE,
			HeaderValue::from_static(mime::APPLICATION_OCTET_STREAM.as_ref()),
		);

		response
	}
}

// --------------------------------------------------
// Body

impl IntoResponse for Body {
	#[inline]
	fn into_response(self) -> Response {
		Response::new(self)
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

// --------------------------------------------------
// Tuples

macro_rules! impl_into_response_for_tuples {
	($t1:ident, $(($($t:ident),*),)? $tl:ident) => {
		#[allow(non_snake_case)]
		impl<$t1, $($($t,)*)? $tl> IntoResponseHeadParts for ($t1, $($($t,)*)? $tl)
		where
			$t1: IntoResponseHeadParts,
			$($($t: IntoResponseHeadParts,)*)?
			$tl: IntoResponseHeadParts,
		{
			fn into_response_head(
				self,
				mut head: ResponseHeadParts,
			) -> Result<ResponseHeadParts, BoxedErrorResponse> {
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
			$($($t: IntoResponseHeadParts,)*)?
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
			$t1: IntoResponseHeadParts,
			$($($t: IntoResponseHeadParts,)*)?
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
