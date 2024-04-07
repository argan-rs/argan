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
	request::RequestHeadParts,
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
	response::{IntoResponse, IntoResponseHead, Response, ResponseError, ResponseHeadParts},
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

	async fn from_request(head_parts: &mut RequestHeadParts, body: B) -> Result<Self, Self::Error> {
		request_into_text_data(&head_parts, body, SIZE_LIMIT)
			.await
			.map(Self)
	}
}

#[inline(always)]
pub(crate) async fn request_into_text_data<B>(
	head_parts: &RequestHeadParts,
	body: B,
	size_limit: usize,
) -> Result<String, TextExtractorError>
where
	B: HttpBody,
	B::Error: Into<BoxedError>,
{
	let content_type = content_type(head_parts)?;

	if content_type == mime::TEXT_PLAIN_UTF_8 || content_type == mime::TEXT_PLAIN {
		match Limited::new(body, size_limit).collect().await {
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

	async fn from_request(head_parts: &mut RequestHeadParts, body: B) -> Result<Self, Self::Error> {
		request_into_binary_data(head_parts, body, SIZE_LIMIT)
			.await
			.map(Self)
	}
}

#[inline(always)]
pub(crate) async fn request_into_binary_data<B>(
	head_parts: &RequestHeadParts,
	body: B,
	size_limit: usize,
) -> Result<Bytes, BinaryExtractorError>
where
	B: HttpBody,
	B::Error: Into<BoxedError>,
{
	let content_type = content_type(head_parts)?;

	if content_type == mime::APPLICATION_OCTET_STREAM || content_type == mime::OCTET_STREAM {
		match Limited::new(body, size_limit).collect().await {
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

	async fn from_request(head_parts: &mut RequestHeadParts, body: B) -> Result<Self, Self::Error> {
		request_into_full_body(body, SIZE_LIMIT).await.map(Self)
	}
}

#[inline(always)]
pub(crate) async fn request_into_full_body<B>(
	body: B,
	size_limit: usize,
) -> Result<Bytes, FullBodyExtractorError>
where
	B: HttpBody,
	B::Error: Into<BoxedError>,
{
	match Limited::new(body, size_limit).collect().await {
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
		let test_body = "Hello, World!".to_string();

		let (mut head_parts, body) = Request::builder()
			.header(
				CONTENT_TYPE,
				HeaderValue::from_static(mime::TEXT_PLAIN_UTF_8.as_ref()),
			)
			.body(test_body.clone())
			.unwrap()
			.into_parts();

		let Text(data) = Text::<1024>::from_request(&mut head_parts, body)
			.await
			.unwrap();

		assert_eq!(data, test_body.as_ref());

		// ----------

		let (mut head_parts, body) = Request::new(test_body.clone()).into_parts();

		let error = Text::<1024>::from_request(&mut head_parts, body)
			.await
			.unwrap_err();

		match error {
			TextExtractorError::MissingContentType => {}
			error => panic!("unexpected error {}", error),
		}

		// ----------

		let (mut head_parts, body) = Request::builder()
			.header(
				CONTENT_TYPE,
				HeaderValue::from_static(mime::OCTET_STREAM.as_ref()),
			)
			.body(test_body.clone())
			.unwrap()
			.into_parts();

		let error = Text::<1024>::from_request(&mut head_parts, body)
			.await
			.unwrap_err();

		match error {
			TextExtractorError::UnsupportedMediaType => {}
			error => panic!("unexpected error {}", error),
		}

		// ----------

		let (mut head_parts, body) = Request::builder()
			.header(
				CONTENT_TYPE,
				HeaderValue::from_static(mime::TEXT_PLAIN_UTF_8.as_ref()),
			)
			.body(test_body.clone())
			.unwrap()
			.into_parts();

		let error = Text::<8>::from_request(&mut head_parts, body)
			.await
			.unwrap_err();

		match error {
			TextExtractorError::ContentTooLarge => {}
			error => panic!("unexpected error {}", error),
		}
	}

	#[tokio::test]
	async fn binary_extractor() {
		let test_body = &b"Hello, World!"[..];
		let full_body = Full::new(test_body);

		let (mut head_parts, body) = Request::builder()
			.header(
				CONTENT_TYPE,
				HeaderValue::from_static(mime::APPLICATION_OCTET_STREAM.as_ref()),
			)
			.body(full_body.clone())
			.unwrap()
			.into_parts();

		let Binary(data) = Binary::<1024>::from_request(&mut head_parts, body)
			.await
			.unwrap();

		assert_eq!(&data, test_body);

		// ----------

		let (mut head_parts, body) = Request::new(full_body.clone()).into_parts();

		let error = Binary::<1024>::from_request(&mut head_parts, body)
			.await
			.unwrap_err();

		match error {
			BinaryExtractorError::MissingContentType => {}
			error => panic!("unexpected error {}", error),
		}

		// ----------

		let full_body = Full::new(test_body);
		let (mut head_parts, body) = Request::builder()
			.header(
				CONTENT_TYPE,
				HeaderValue::from_static(mime::TEXT_PLAIN.as_ref()),
			)
			.body(full_body.clone())
			.unwrap()
			.into_parts();

		let error = Binary::<1024>::from_request(&mut head_parts, body)
			.await
			.unwrap_err();

		match error {
			BinaryExtractorError::UnsupportedMediaType => {}
			error => panic!("unexpected error {}", error),
		}

		// ----------

		let full_body = Full::new(test_body);
		let (mut head_parts, body) = Request::builder()
			.header(
				CONTENT_TYPE,
				HeaderValue::from_static(mime::APPLICATION_OCTET_STREAM.as_ref()),
			)
			.body(full_body.clone())
			.unwrap()
			.into_parts();

		let error = Binary::<8>::from_request(&mut head_parts, body)
			.await
			.unwrap_err();

		match error {
			BinaryExtractorError::ContentTooLarge => {}
			error => panic!("unexpected error {}", error),
		}
	}

	#[tokio::test]
	async fn full_body_bytes_extractor() {
		let test_body = &b"Hello, World!"[..];
		let full_body = Full::new(test_body);

		let (mut head_parts, body) = Request::builder()
			.header(
				CONTENT_TYPE,
				HeaderValue::from_static(mime::TEXT_PLAIN_UTF_8.as_ref()),
			)
			.body(full_body.clone())
			.unwrap()
			.into_parts();

		let FullBody(data) = FullBody::<1024>::from_request(&mut head_parts, body)
			.await
			.unwrap();

		assert_eq!(&data, test_body);

		// ----------

		let (mut head_parts, body) = Request::builder()
			.header(
				CONTENT_TYPE,
				HeaderValue::from_static(mime::APPLICATION_OCTET_STREAM.as_ref()),
			)
			.body(full_body.clone())
			.unwrap()
			.into_parts();

		let FullBody(data) = FullBody::<1024>::from_request(&mut head_parts, body)
			.await
			.unwrap();

		assert_eq!(&data, test_body);

		// ----------

		let (mut head_parts, body) = Request::new(full_body.clone()).into_parts();

		let FullBody(data) = FullBody::<1024>::from_request(&mut head_parts, body)
			.await
			.unwrap();

		assert_eq!(&data, test_body);

		// ----------

		let full_body = Full::new(test_body);
		let (mut head_parts, body) = Request::builder()
			.header(
				CONTENT_TYPE,
				HeaderValue::from_static(mime::APPLICATION_OCTET_STREAM.as_ref()),
			)
			.body(full_body.clone())
			.unwrap()
			.into_parts();

		let error = FullBody::<8>::from_request(&mut head_parts, body)
			.await
			.unwrap_err();

		match error {
			FullBodyExtractorError::ContentTooLarge => {}
			error => panic!("unexpected error {}", error),
		}

		// ----------

		let (mut head_parts, body) = Request::new(full_body.clone()).into_parts();

		let error = FullBody::<8>::from_request(&mut head_parts, body)
			.await
			.unwrap_err();

		match error {
			FullBodyExtractorError::ContentTooLarge => {}
			error => panic!("unexpected error {}", error),
		}
	}
}
