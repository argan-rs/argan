use std::{
	borrow::Cow,
	convert::Infallible,
	fmt::Debug,
	future::{ready, Future, Ready},
	marker::PhantomData,
	pin::{pin, Pin},
	task::{Context, Poll},
};

use cookie::{Cookie, CookieJar};
use http::{
	header::{CONTENT_TYPE, COOKIE, SET_COOKIE},
	StatusCode, Version,
};
use http_body_util::{BodyExt, Empty, Full, Limited};
use hyper::body::{Body, Bytes};
use pin_project::pin_project;
use serde::{de::DeserializeOwned, Serialize};

use crate::{
	request::{content_type, FromRequest, FromRequestHead, Request, RequestHead},
	response::{IntoResponse, IntoResponseHead, Response, ResponseHead},
	utils::BoxedError,
};

// ----------

pub use http::{header, Extensions, HeaderMap, HeaderName, HeaderValue};

// --------------------------------------------------

pub mod form;

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

// --------------------------------------------------
// Json

pub struct Json<T, const SIZE_LIMIT: usize = { 2 * 1024 * 1024 }>(pub T);

impl<B, T, const SIZE_LIMIT: usize> FromRequest<B> for Json<T, SIZE_LIMIT>
where
	B: Body + Send,
	B::Data: Send,
	B::Error: Into<BoxedError>,
	T: DeserializeOwned,
{
	type Error = StatusCode; // TODO.

	async fn from_request(request: Request<B>) -> Result<Self, Self::Error> {
		let content_type = content_type(&request)?;

		if content_type == mime::APPLICATION_JSON {
			match Limited::new(request, SIZE_LIMIT).collect().await {
				Ok(body) => match serde_json::from_slice::<T>(&body.to_bytes()) {
					Ok(value) => Ok(Json(value)),
					Err(_) => Err(StatusCode::BAD_REQUEST),
				},
				Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
			}
		} else {
			Err(StatusCode::UNSUPPORTED_MEDIA_TYPE)
		}
	}
}

impl<T> IntoResponse for Json<T>
where
	T: Serialize,
{
	fn into_response(self) -> Response {
		let mut response = serde_json::to_string(&self.0).unwrap().into_response();
		response.headers_mut().insert(
			CONTENT_TYPE,
			HeaderValue::from_static(mime::APPLICATION_JSON.as_ref()),
		);

		response
	}
}

// --------------------------------------------------
// Cookies

pub struct Cookies(CookieJar);

impl FromRequestHead for Cookies {
	type Error = Infallible;

	async fn from_request_head(head: &mut RequestHead) -> Result<Self, Self::Error> {
		let cookie_jar = head
			.headers
			.get_all(COOKIE)
			.iter()
			.filter_map(|value| value.to_str().ok())
			.flat_map(Cookie::split_parse_encoded)
			.fold(CookieJar::new(), |mut jar, result| {
				match result {
					Ok(cookie) => jar.add_original(cookie.into_owned()),
					Err(_) => {} // TODO.
				}

				jar
			});

		Ok(Cookies(cookie_jar))
	}
}

impl IntoResponseHead for Cookies {
	type Error = Infallible;

	fn into_response_head(self, mut head: ResponseHead) -> Result<ResponseHead, Self::Error> {
		for cookie in self.0.delta() {
			match HeaderValue::try_from(cookie.encoded().to_string()) {
				Ok(header_value) => head.headers.append(SET_COOKIE, header_value),
				Err(_) => todo!(),
			};
		}

		Ok(head)
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

impl<B> FromRequest<B> for String
where
	B: Body + Send,
	B::Data: Send,
	B::Error: Debug,
{
	type Error = StatusCode; // TODO.

	async fn from_request(request: Request<B>) -> Result<Self, Self::Error> {
		let content_type = content_type(&request)?;

		if content_type == mime::TEXT_PLAIN_UTF_8 {
			match request.collect().await {
				Ok(body) => match String::from_utf8(body.to_bytes().into()) {
					Ok(text) => Ok(text),
					Err(error) => Err(StatusCode::BAD_REQUEST),
				},
				Err(error) => Err(StatusCode::INTERNAL_SERVER_ERROR),
			}
		} else {
			Err(StatusCode::UNSUPPORTED_MEDIA_TYPE)
		}
	}
}

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
			CONTENT_TYPE,
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
			CONTENT_TYPE,
			HeaderValue::from_static(mime::APPLICATION_OCTET_STREAM.as_ref()),
		);

		response
	}
}

// --------------------------------------------------
// Bytes

impl<B> FromRequest<B> for Bytes
where
	B: Body + Send,
	B::Data: Send,
	B::Error: Debug,
{
	type Error = StatusCode; // TODO.

	async fn from_request(request: Request<B>) -> Result<Self, Self::Error> {
		let content_type_str = content_type(&request)?;

		if content_type_str == mime::APPLICATION_OCTET_STREAM {
			match request.collect().await {
				Ok(body) => Ok(body.to_bytes()),
				Err(error) => Err(StatusCode::INTERNAL_SERVER_ERROR),
			}
		} else {
			Err(StatusCode::UNSUPPORTED_MEDIA_TYPE)
		}
	}
}

impl IntoResponse for Bytes {
	#[inline]
	fn into_response(self) -> Response {
		let mut response = Full::from(self).into_response();
		response.headers_mut().insert(
			CONTENT_TYPE,
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
		Response::new(self.map_err(Into::into).boxed())
	}
}

// --------------------------------------------------
// Full<Bytes>

impl IntoResponse for Full<Bytes> {
	#[inline]
	fn into_response(self) -> Response {
		Response::new(self.map_err(Into::into).boxed())
	}
}

// --------------------------------------------------------------------------------
