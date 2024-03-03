use std::{
	any,
	borrow::Cow,
	convert::Infallible,
	fmt::Debug,
	future::{ready, Future, Ready},
	marker::PhantomData,
	pin::{pin, Pin},
	string::FromUtf8Error,
	task::{Context, Poll},
};

use http::{
	header::{ToStrError, CONTENT_TYPE, COOKIE, SET_COOKIE},
	HeaderValue, StatusCode, Version,
};
use http_body_util::{BodyExt, Empty, Full, LengthLimitError, Limited};
use pin_project::pin_project;
use serde::{de::DeserializeOwned, Serialize};
use serde_json::error::Category;

use crate::{
	body::{Body, Bytes, HttpBody},
	common::BoxedError,
	handler::Args,
	header::{content_type, ContentTypeError},
	request::{FromRequest, FromRequestHead, Request, RequestHead},
	response::{IntoResponse, IntoResponseHead, Response, ResponseError, ResponseHead},
	ImplError,
};

// --------------------------------------------------

pub mod cookie;
pub mod extensions;
pub mod form;
pub mod json;

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

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

#[derive(Debug)]
pub struct Text<const SIZE_LIMIT: usize = { 2 * 1024 * 1024 }>(String);

impl<B, E, const SIZE_LIMIT: usize> FromRequest<B, E> for Text<SIZE_LIMIT>
where
	B: HttpBody + Send,
	B::Data: Send,
	B::Error: Into<BoxedError>,
	E: Sync,
{
	type Error = TextExtractorError;

	async fn from_request(request: Request<B>, _args: &mut Args<'_, E>) -> Result<Self, Self::Error> {
		let content_type = content_type(&request)?;

		if content_type == mime::TEXT_PLAIN_UTF_8 || content_type == mime::TEXT_PLAIN {
			match Limited::new(request, SIZE_LIMIT).collect().await {
				Ok(body) => String::from_utf8(body.to_bytes().into())
					.map(|value| Self(value))
					.map_err(Into::<TextExtractorError>::into),
				Err(error) => Err(
					error
						.downcast_ref::<LengthLimitError>()
						.map_or(TextExtractorError::BufferingFailure, |_| {
							TextExtractorError::ContentTooLarge
						}),
				),
			}
		} else {
			Err(TextExtractorError::UnsupportedMediaType)
		}
	}
}

impl IntoResponse for String {
	#[inline]
	fn into_response(self) -> Response {
		Cow::<'_, str>::Owned(self).into_response()
	}
}

// ----------

data_extractor_error! {
	#[derive(Debug)]
	pub TextExtractorError {
		#[error("decoding failure: {0}")]
		(DecodingFailure(#[from] FromUtf8Error)) [(_)]; StatusCode::BAD_REQUEST;
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
// Octets

#[derive(Debug)]
pub struct Octets<const SIZE_LIMIT: usize = { 16 * 1024 * 1024 }>(Bytes);

impl<B, E, const SIZE_LIMIT: usize> FromRequest<B, E> for Octets<SIZE_LIMIT>
where
	B: HttpBody + Send,
	B::Data: Send,
	B::Error: Into<BoxedError>,
	E: Sync,
{
	type Error = OctetsExtractorError;

	async fn from_request(request: Request<B>, _args: &mut Args<'_, E>) -> Result<Self, Self::Error> {
		let content_type = content_type(&request)?;

		if content_type == mime::APPLICATION_OCTET_STREAM || content_type == mime::OCTET_STREAM {
			match Limited::new(request, SIZE_LIMIT).collect().await {
				Ok(body) => Ok(Octets(body.to_bytes())),
				Err(error) => Err(
					error
						.downcast_ref::<LengthLimitError>()
						.map_or(OctetsExtractorError::BufferingFailure, |_| {
							OctetsExtractorError::ContentTooLarge
						}),
				),
			}
		} else {
			Err(OctetsExtractorError::UnsupportedMediaType)
		}
	}
}

// ----------

data_extractor_error! {
	#[derive(Debug)]
	pub OctetsExtractorError {}
}

// --------------------------------------------------
// RawBody

#[derive(Debug)]
pub struct RawBody<const SIZE_LIMIT: usize = { 16 * 1024 * 1024 }>(Bytes);

impl<B, E, const SIZE_LIMIT: usize> FromRequest<B, E> for RawBody<SIZE_LIMIT>
where
	B: HttpBody + Send,
	B::Data: Send,
	B::Error: Into<BoxedError>,
	E: Sync,
{
	type Error = RawBodyExtractorError;

	async fn from_request(request: Request<B>, _args: &mut Args<'_, E>) -> Result<Self, Self::Error> {
		match Limited::new(request, SIZE_LIMIT).collect().await {
			Ok(body) => Ok(RawBody(body.to_bytes())),
			Err(error) => Err(
				error
					.downcast_ref::<LengthLimitError>()
					.map_or(RawBodyExtractorError::BufferingFailure, |_| {
						RawBodyExtractorError::ContentTooLarge
					}),
			),
		}
	}
}

// ----------

#[non_exhaustive]
#[derive(Debug, crate::ImplError)]
pub enum RawBodyExtractorError {
	#[error("content too large")]
	ContentTooLarge,
	#[error("buffering failure")]
	BufferingFailure,
}

impl IntoResponse for RawBodyExtractorError {
	fn into_response(self) -> Response {
		match self {
			Self::ContentTooLarge => StatusCode::PAYLOAD_TOO_LARGE.into_response(),
			Self::BufferingFailure => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
		}
	}
}

// --------------------------------------------------
// Bytes

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
