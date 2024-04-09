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
