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

use argan_core::{
	body::{Body, HttpBody},
	BoxedError,
};
use bytes::Bytes;
use http::{
	header::{ToStrError, CONTENT_TYPE, COOKIE, SET_COOKIE},
	HeaderValue, StatusCode, Version,
};
use http_body_util::{BodyExt, Empty, Full, LengthLimitError, Limited};
use pin_project::pin_project;
use serde::{de::DeserializeOwned, Serialize};
use serde_json::error::Category;

use crate::{
	handler::Args,
	request::{FromRequest, Request, RequestHead},
	response::{IntoResponse, IntoResponseHead, Response, ResponseError, ResponseHead},
	routing::RoutingState,
	ImplError,
};

// --------------------------------------------------

pub mod cookie;
pub mod extensions;
pub mod form;
pub mod header;
pub mod json;

use header::content_type;

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

// --------------------------------------------------

pub(crate) const BODY_SIZE_LIMIT: usize = { 2 * 1024 * 1024 };

// --------------------------------------------------
// String

#[derive(Debug)]
pub struct Text<const SIZE_LIMIT: usize = BODY_SIZE_LIMIT>(String);

impl<B, const SIZE_LIMIT: usize> FromRequest<B> for Text<SIZE_LIMIT>
where
	B: HttpBody + Send,
	B::Data: Send,
	B::Error: Into<BoxedError>,
{
	type Error = TextExtractorError;

	async fn from_request(request: Request<B>) -> Result<Self, Self::Error> {
		request_into_text_data(request, SIZE_LIMIT).await.map(Self)
	}
}

// ----------

#[inline(always)]
pub(crate) async fn request_into_text_data<B>(
	request: Request<B>,
	size_limit: usize,
) -> Result<String, TextExtractorError>
where
	B: HttpBody,
	B::Error: Into<BoxedError>,
{
	let content_type = content_type(&request)?;

	if content_type == mime::TEXT_PLAIN_UTF_8 || content_type == mime::TEXT_PLAIN {
		match Limited::new(request, size_limit).collect().await {
			Ok(body) => String::from_utf8(body.to_bytes().into())
				.map(|value| value)
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

// ----------

data_extractor_error! {
	#[derive(Debug)]
	pub TextExtractorError {
		#[error("decoding failure: {0}")]
		(DecodingFailure(#[from] FromUtf8Error)) [(_)]; StatusCode::BAD_REQUEST;
	}
}

// --------------------------------------------------
// Binary

#[derive(Debug)]
pub struct Binary<const SIZE_LIMIT: usize = BODY_SIZE_LIMIT>(Bytes);

impl<B, const SIZE_LIMIT: usize> FromRequest<B> for Binary<SIZE_LIMIT>
where
	B: HttpBody + Send,
	B::Data: Send,
	B::Error: Into<BoxedError>,
{
	type Error = BinaryExtractorError;

	async fn from_request(request: Request<B>) -> Result<Self, Self::Error> {
		request_into_binary_data(request, SIZE_LIMIT)
			.await
			.map(Self)
	}
}

// ----------

#[inline(always)]
pub(crate) async fn request_into_binary_data<B>(
	request: Request<B>,
	size_limit: usize,
) -> Result<Bytes, BinaryExtractorError>
where
	B: HttpBody,
	B::Error: Into<BoxedError>,
{
	let content_type = content_type(&request)?;

	if content_type == mime::APPLICATION_OCTET_STREAM || content_type == mime::OCTET_STREAM {
		match Limited::new(request, size_limit).collect().await {
			Ok(body) => Ok(body.to_bytes()),
			Err(error) => Err(
				error
					.downcast_ref::<LengthLimitError>()
					.map_or(BinaryExtractorError::BufferingFailure, |_| {
						BinaryExtractorError::ContentTooLarge
					}),
			),
		}
	} else {
		Err(BinaryExtractorError::UnsupportedMediaType)
	}
}

// ----------

data_extractor_error! {
	#[derive(Debug)]
	pub BinaryExtractorError {}
}

// --------------------------------------------------
// FullBody

#[derive(Debug)]
pub struct FullBody<const SIZE_LIMIT: usize = BODY_SIZE_LIMIT>(Bytes);

impl<B, const SIZE_LIMIT: usize> FromRequest<B> for FullBody<SIZE_LIMIT>
where
	B: HttpBody + Send,
	B::Data: Send,
	B::Error: Into<BoxedError>,
{
	type Error = FullBodyExtractorError;

	async fn from_request(request: Request<B>) -> Result<Self, Self::Error> {
		request_into_full_body(request, SIZE_LIMIT).await.map(Self)
	}
}

// ----------

#[inline(always)]
pub(crate) async fn request_into_full_body<B>(
	request: Request<B>,
	size_limit: usize,
) -> Result<Bytes, FullBodyExtractorError>
where
	B: HttpBody,
	B::Error: Into<BoxedError>,
{
	match Limited::new(request, size_limit).collect().await {
		Ok(body) => Ok(body.to_bytes()),
		Err(error) => Err(
			error
				.downcast_ref::<LengthLimitError>()
				.map_or(FullBodyExtractorError::BufferingFailure, |_| {
					FullBodyExtractorError::ContentTooLarge
				}),
		),
	}
}

// ----------

#[non_exhaustive]
#[derive(Debug, crate::ImplError)]
pub enum FullBodyExtractorError {
	#[error("content too large")]
	ContentTooLarge,
	#[error("buffering failure")]
	BufferingFailure,
}

impl IntoResponse for FullBodyExtractorError {
	fn into_response(self) -> Response {
		match self {
			Self::ContentTooLarge => StatusCode::PAYLOAD_TOO_LARGE.into_response(),
			Self::BufferingFailure => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
		}
	}
}

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

#[cfg(test)]
mod test {
	use argan_core::body::HttpBody;
	use http::Extensions;

	use crate::routing::{RouteTraversal, RoutingState};

	use super::*;

	// --------------------------------------------------------------------------------
	// --------------------------------------------------------------------------------

	#[tokio::test]
	async fn text_extractor() {
		let body = "Hello, World!".to_string();

		let request = Request::builder()
			.header(
				CONTENT_TYPE,
				HeaderValue::from_static(mime::TEXT_PLAIN_UTF_8.as_ref()),
			)
			.body(body.clone())
			.unwrap();

		let Text(data) = Text::<1024>::from_request(request).await.unwrap();

		assert_eq!(data, body.as_ref());

		// ----------

		let request = Request::new(body.clone());

		let error = Text::<1024>::from_request(request).await.unwrap_err();

		match error {
			TextExtractorError::MissingContentType => {}
			error => panic!("unexpected error {}", error),
		}

		// ----------

		let request = Request::builder()
			.header(
				CONTENT_TYPE,
				HeaderValue::from_static(mime::OCTET_STREAM.as_ref()),
			)
			.body(body.clone())
			.unwrap();

		let error = Text::<1024>::from_request(request).await.unwrap_err();

		match error {
			TextExtractorError::UnsupportedMediaType => {}
			error => panic!("unexpected error {}", error),
		}

		// ----------

		let request = Request::builder()
			.header(
				CONTENT_TYPE,
				HeaderValue::from_static(mime::TEXT_PLAIN_UTF_8.as_ref()),
			)
			.body(body.clone())
			.unwrap();

		let error = Text::<8>::from_request(request).await.unwrap_err();

		match error {
			TextExtractorError::ContentTooLarge => {}
			error => panic!("unexpected error {}", error),
		}
	}

	#[tokio::test]
	async fn binary_extractor() {
		let body = &b"Hello, World!"[..];
		let full_body = Full::new(body);
		let request = Request::builder()
			.header(
				CONTENT_TYPE,
				HeaderValue::from_static(mime::APPLICATION_OCTET_STREAM.as_ref()),
			)
			.body(full_body.clone())
			.unwrap();

		let Binary(data) = Binary::<1024>::from_request(request).await.unwrap();

		assert_eq!(&data, body);

		// ----------

		let request = Request::new(full_body.clone());

		let error = Binary::<1024>::from_request(request).await.unwrap_err();

		match error {
			BinaryExtractorError::MissingContentType => {}
			error => panic!("unexpected error {}", error),
		}

		// ----------

		let full_body = Full::new(body);
		let request = Request::builder()
			.header(
				CONTENT_TYPE,
				HeaderValue::from_static(mime::TEXT_PLAIN.as_ref()),
			)
			.body(full_body.clone())
			.unwrap();

		let error = Binary::<1024>::from_request(request).await.unwrap_err();

		match error {
			BinaryExtractorError::UnsupportedMediaType => {}
			error => panic!("unexpected error {}", error),
		}

		// ----------

		let full_body = Full::new(body);
		let request = Request::builder()
			.header(
				CONTENT_TYPE,
				HeaderValue::from_static(mime::APPLICATION_OCTET_STREAM.as_ref()),
			)
			.body(full_body.clone())
			.unwrap();

		let error = Binary::<8>::from_request(request).await.unwrap_err();

		match error {
			BinaryExtractorError::ContentTooLarge => {}
			error => panic!("unexpected error {}", error),
		}
	}

	#[tokio::test]
	async fn full_body_bytes_extractor() {
		let body = &b"Hello, World!"[..];
		let full_body = Full::new(body);

		let request = Request::builder()
			.header(
				CONTENT_TYPE,
				HeaderValue::from_static(mime::TEXT_PLAIN_UTF_8.as_ref()),
			)
			.body(full_body.clone())
			.unwrap();

		let FullBody(data) = FullBody::<1024>::from_request(request).await.unwrap();

		assert_eq!(&data, body);

		// ----------

		let request = Request::builder()
			.header(
				CONTENT_TYPE,
				HeaderValue::from_static(mime::APPLICATION_OCTET_STREAM.as_ref()),
			)
			.body(full_body.clone())
			.unwrap();

		let FullBody(data) = FullBody::<1024>::from_request(request).await.unwrap();

		assert_eq!(&data, body);

		// ----------

		let request = Request::new(full_body.clone());

		let FullBody(data) = FullBody::<1024>::from_request(request).await.unwrap();

		assert_eq!(&data, body);

		// ----------

		let full_body = Full::new(body);
		let request = Request::builder()
			.header(
				CONTENT_TYPE,
				HeaderValue::from_static(mime::APPLICATION_OCTET_STREAM.as_ref()),
			)
			.body(full_body.clone())
			.unwrap();

		let error = FullBody::<8>::from_request(request).await.unwrap_err();

		match error {
			FullBodyExtractorError::ContentTooLarge => {}
			error => panic!("unexpected error {}", error),
		}

		// ----------

		let request = Request::new(full_body.clone());

		let error = FullBody::<8>::from_request(request).await.unwrap_err();

		match error {
			FullBodyExtractorError::ContentTooLarge => {}
			error => panic!("unexpected error {}", error),
		}
	}
}
