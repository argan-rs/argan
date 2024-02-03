use std::{any::Any, convert::Infallible};

use http::{header::LOCATION, response::Parts, HeaderValue};

use crate::{
	body::{Body, BodyExt, Bytes, HttpBody},
	common::{BoxedError, SCOPE_VALIDITY},
	request::FromRequestHead,
};

// ----------

pub use http::StatusCode;

// --------------------------------------------------

pub mod stream;

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

pub type Response<B = Body> = http::response::Response<B>;
pub type ResponseHead = Parts;

// --------------------------------------------------
// IntoResponseHead trait

pub trait IntoResponseHead {
	type Error: IntoResponse;

	fn into_response_head(self, head: ResponseHead) -> Result<ResponseHead, Self::Error>;
}

// --------------------------------------------------
// IntoResponse trait

pub trait IntoResponse {
	fn into_response(self) -> Response;
}

impl<B> IntoResponse for Response<B>
where
	B: HttpBody<Data = Bytes> + Send + Sync + 'static,
	B::Error: Into<BoxedError>,
{
	fn into_response(self) -> Response {
		let (head, body) = self.into_parts();
		let boxed_body = Body::new(body);

		Response::from_parts(head, boxed_body)
	}
}

// --------------------------------------------------
// ResponseHead

// impl IntoResponse for ResponseHead {
// 	fn into_response(self) -> Response {
// 		Response::from_parts(self, Bytes::default())
// 	}
// }

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
// Result<T, E>

impl<T, E> IntoResponse for Result<T, E>
where
	T: IntoResponse,
	E: IntoResponse,
{
	#[inline]
	fn into_response(self) -> Response {
		match self {
			Ok(value) => value.into_response(),
			Err(error) => error.into_response(),
		}
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
			type Error = Response;

			fn into_response_head(self, mut head: ResponseHead) -> Result<ResponseHead, Self::Error> {
				let ($t1, $($($t,)*)? $tl) = self;

				head = $t1.into_response_head(head).map_err(|error| error.into_response())?;

				$($(head = $t.into_response_head(head).map_err(|error| error.into_response())?;)*)?

				head = $tl.into_response_head(head).map_err(|error| error.into_response())?;

				Ok(head)
			}
		}

		#[allow(non_snake_case)]
		impl<$($($t,)*)? $tl> IntoResponse for (StatusCode, $($($t,)*)? $tl)
		where
			$($($t: IntoResponseHead,)*)?
			$tl: IntoResponse,
		{
			fn into_response(self) -> Response {
				let (status_code, $($($t,)*)? $tl) = self;

				let (head, body) = $tl.into_response().into_parts();

				$($(
					let head = match $t.into_response_head(head) {
						Ok(head) => head,
						Err(error) => return error.into_response(),
					};
				)*)?

				let mut response = Response::from_parts(head, body);
				*response.status_mut() = status_code;

				response
			}
		}

		#[allow(non_snake_case)]
		impl<$t1, $($($t,)*)? $tl> IntoResponse for ($t1, $($($t,)*)? $tl)
		where
			$t1: IntoResponseHead,
			$($($t: IntoResponseHead,)*)?
			$tl: IntoResponse,
		{
			fn into_response(self) -> Response {
				let ($t1, $($($t,)*)? $tl) = self;

				let (head, body) = $tl.into_response().into_parts();

				let head = match $t1.into_response_head(head) {
					Ok(head) => head,
					Err(error) => return error.into_response(),
				};

				$($(
					let head = match $t.into_response_head(head) {
						Ok(head) => head,
						Err(error) => return error.into_response(),
					};
				)*)?

				Response::from_parts(head, body)
			}
		}
	};
}

call_for_tuples!(impl_into_response_for_tuples!);

// --------------------------------------------------------------------------------
