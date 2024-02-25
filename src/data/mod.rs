use std::{
	any,
	borrow::Cow,
	convert::Infallible,
	fmt::Debug,
	future::{ready, Future, Ready},
	marker::PhantomData,
	pin::{pin, Pin},
	task::{Context, Poll},
};

use http::{
	header::{ToStrError, CONTENT_TYPE, COOKIE, SET_COOKIE},
	StatusCode, Version,
};
use http_body_util::{BodyExt, Empty, Full, LengthLimitError, Limited};
use pin_project::pin_project;
use serde::{de::DeserializeOwned, Serialize};
use serde_json::error::Category;

use crate::{
	body::{Body, Bytes, HttpBody},
	common::BoxedError,
	handler::Args,
	header::ContentTypeError,
	request::{content_type, FromRequest, FromRequestHead, Request, RequestHead},
	response::{IntoResponse, IntoResponseHead, Response, ResponseError, ResponseHead},
	ImplError,
};

// ----------

pub use http::{header, HeaderMap, HeaderName, HeaderValue};

// --------------------------------------------------

pub mod cookie;
pub mod extensions;
pub mod form;

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

// --------------------------------------------------
// Json

pub struct Json<T, const SIZE_LIMIT: usize = { 2 * 1024 * 1024 }>(pub T);

impl<B, E, T, const SIZE_LIMIT: usize> FromRequest<B, E> for Json<T, SIZE_LIMIT>
where
	B: HttpBody + Send,
	B::Data: Send,
	B::Error: Into<BoxedError>,
	E: Sync,
	T: DeserializeOwned,
{
	type Error = JsonError;

	async fn from_request(request: Request<B>, _args: &mut Args<'_, E>) -> Result<Self, Self::Error> {
		let content_type = content_type(&request)?;

		if content_type == mime::APPLICATION_JSON {
			match Limited::new(request, SIZE_LIMIT).collect().await {
				Ok(body) => Ok(
					serde_json::from_slice::<T>(&body.to_bytes())
						.map(|value| Self(value))
						.map_err(Into::<JsonError>::into)?,
				),
				Err(error) => Err(
					error
						.downcast_ref::<LengthLimitError>()
						.map_or(JsonError::BufferingFailure, |_| JsonError::ContentTooLarge),
				),
			}
		} else {
			Err(JsonError::UnsupportedMediaType)
		}
	}
}

impl<T> IntoResponse for Json<T>
where
	T: Serialize,
{
	fn into_response(self) -> Response {
		let json_string = match serde_json::to_string(&self.0).map_err(Into::<JsonError>::into) {
			Ok(json_string) => json_string,
			Err(error) => return error.into_response(),
		};

		let mut response = json_string.into_response();
		response.headers_mut().insert(
			CONTENT_TYPE,
			HeaderValue::from_static(mime::APPLICATION_JSON.as_ref()),
		);

		response
	}
}

// ----------

data_extractor_error! {
	#[derive(Debug)]
	pub JsonError {
		#[error("invlaid JSON syntax in line {line}, column {column}")]
		(InvalidSyntax { line: usize, column: usize}) [{..}]; StatusCode::BAD_REQUEST;
		#[error("invalid JSON semantics in line {line}, column {column}")]
		(InvalidData { line: usize, column: usize}) [{..}]; StatusCode::UNPROCESSABLE_ENTITY;
	}
}

impl From<serde_json::Error> for JsonError {
	fn from(error: serde_json::Error) -> Self {
		match error.classify() {
			Category::Syntax => JsonError::InvalidSyntax {
				line: error.line(),
				column: error.column(),
			},
			Category::Data => JsonError::InvalidData {
				line: error.line(),
				column: error.column(),
			},
			_ => JsonError::BufferingFailure,
		}
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

pub struct Text<const SIZE_LIMIT: usize = { 2 * 1024 * 1024 }>(String);

impl<B, E, const SIZE_LIMIT: usize> FromRequest<B, E> for Text<SIZE_LIMIT>
where
	B: HttpBody + Send,
	B::Data: Send,
	B::Error: Into<BoxedError>,
	E: Sync,
{
	type Error = ResponseError; // TODO.

	async fn from_request(request: Request<B>, _args: &mut Args<'_, E>) -> Result<Self, Self::Error> {
		let content_type = content_type(&request).map_err(|_| StatusCode::BAD_REQUEST)?;

		if content_type == mime::TEXT_PLAIN_UTF_8 {
			match Limited::new(request, SIZE_LIMIT).collect().await {
				Ok(body) => match String::from_utf8(body.to_bytes().into()) {
					Ok(text) => Ok(Text(text)),
					Err(error) => Err(StatusCode::BAD_REQUEST.into()),
				},
				Err(error) => Err(
					error
						.downcast_ref::<LengthLimitError>()
						.map_or(StatusCode::INTERNAL_SERVER_ERROR, |_| {
							StatusCode::PAYLOAD_TOO_LARGE
						})
						.into(),
				),
			}
		} else {
			Err(StatusCode::UNSUPPORTED_MEDIA_TYPE.into())
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

pub struct Binary<const SIZE_LIMIT: usize = { 16 * 1024 * 1024 }>(Bytes);

impl<B, E, const SIZE_LIMIT: usize> FromRequest<B, E> for Binary<SIZE_LIMIT>
where
	B: HttpBody + Send,
	B::Data: Send,
	B::Error: Into<BoxedError>,
	E: Sync,
{
	type Error = ResponseError; // TODO.

	async fn from_request(request: Request<B>, _args: &mut Args<'_, E>) -> Result<Self, Self::Error> {
		let content_type_str = content_type(&request).map_err(|_| StatusCode::BAD_REQUEST)?;

		if content_type_str == mime::APPLICATION_OCTET_STREAM {
			match Limited::new(request, SIZE_LIMIT).collect().await {
				Ok(body) => Ok(Binary(body.to_bytes())),
				Err(error) => Err(
					error
						.downcast_ref::<LengthLimitError>()
						.map_or(StatusCode::INTERNAL_SERVER_ERROR, |_| {
							StatusCode::PAYLOAD_TOO_LARGE
						})
						.into(),
				),
			}
		} else {
			Err(StatusCode::UNSUPPORTED_MEDIA_TYPE.into())
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

// --------------------------------------------------------------------------------
