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
	request::{FromRequest, FromRequestHead, Request, RequestHead},
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
// String

#[derive(Debug)]
pub struct Text<const SIZE_LIMIT: usize = { 2 * 1024 * 1024 }>(String);

impl<'n, B, HE, const SIZE_LIMIT: usize> FromRequest<B, Args<'n, HE>> for Text<SIZE_LIMIT>
where
	B: HttpBody + Send,
	B::Data: Send,
	B::Error: Into<BoxedError>,
	HE: Sync,
{
	type Error = TextExtractorError;

	async fn from_request(request: Request<B>, _args: Args<'n, HE>) -> Result<Self, Self::Error> {
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
pub struct Binary<const SIZE_LIMIT: usize = { 16 * 1024 * 1024 }>(Bytes);

impl<'n, B, HE, const SIZE_LIMIT: usize> FromRequest<B, Args<'n, HE>> for Binary<SIZE_LIMIT>
where
	B: HttpBody + Send,
	B::Data: Send,
	B::Error: Into<BoxedError>,
	HE: Sync,
{
	type Error = BinaryExtractorError;

	async fn from_request(request: Request<B>, _args: Args<'n, HE>) -> Result<Self, Self::Error> {
		let content_type = content_type(&request)?;

		if content_type == mime::APPLICATION_OCTET_STREAM || content_type == mime::OCTET_STREAM {
			match Limited::new(request, SIZE_LIMIT).collect().await {
				Ok(body) => Ok(Binary(body.to_bytes())),
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
}

// ----------

data_extractor_error! {
	#[derive(Debug)]
	pub BinaryExtractorError {}
}

// --------------------------------------------------
// RawBody

#[derive(Debug)]
pub struct RawBody<const SIZE_LIMIT: usize = { 16 * 1024 * 1024 }>(Bytes);

impl<'n, B, HE, const SIZE_LIMIT: usize> FromRequest<B, Args<'n, HE>> for RawBody<SIZE_LIMIT>
where
	B: HttpBody + Send,
	B::Data: Send,
	B::Error: Into<BoxedError>,
	HE: Sync,
{
	type Error = RawBodyExtractorError;

	async fn from_request(request: Request<B>, _args: Args<'n, HE>) -> Result<Self, Self::Error> {
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

		let Text(data) = Text::<1024>::from_request(request, Args::new())
			.await
			.unwrap();

		assert_eq!(data, body.as_ref());

		// ----------

		let request = Request::new(body.clone());

		let error = Text::<1024>::from_request(request, Args::new())
			.await
			.unwrap_err();

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

		let error = Text::<1024>::from_request(request, Args::new())
			.await
			.unwrap_err();

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

		let error = Text::<8>::from_request(request, Args::new())
			.await
			.unwrap_err();

		match error {
			TextExtractorError::ContentTooLarge => {}
			error => panic!("unexpected error {}", error),
		}
	}

	#[tokio::test]
	async fn octets_extractor() {
		let body = &b"Hello, World!"[..];
		let full_body = Full::new(body);
		let request = Request::builder()
			.header(
				CONTENT_TYPE,
				HeaderValue::from_static(mime::APPLICATION_OCTET_STREAM.as_ref()),
			)
			.body(full_body.clone())
			.unwrap();

		let Binary(data) = Binary::<1024>::from_request(request, Args::new())
			.await
			.unwrap();

		assert_eq!(&data, body);

		// ----------

		let request = Request::new(full_body.clone());

		let error = Binary::<1024>::from_request(request, Args::new())
			.await
			.unwrap_err();

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

		let error = Binary::<1024>::from_request(request, Args::new())
			.await
			.unwrap_err();

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

		let error = Binary::<8>::from_request(request, Args::new())
			.await
			.unwrap_err();

		match error {
			BinaryExtractorError::ContentTooLarge => {}
			error => panic!("unexpected error {}", error),
		}
	}

	#[tokio::test]
	async fn raw_body_extractor() {
		let body = &b"Hello, World!"[..];
		let full_body = Full::new(body);

		let request = Request::builder()
			.header(
				CONTENT_TYPE,
				HeaderValue::from_static(mime::TEXT_PLAIN_UTF_8.as_ref()),
			)
			.body(full_body.clone())
			.unwrap();

		let RawBody(data) = RawBody::<1024>::from_request(request, Args::new())
			.await
			.unwrap();

		assert_eq!(&data, body);

		// ----------

		let request = Request::builder()
			.header(
				CONTENT_TYPE,
				HeaderValue::from_static(mime::APPLICATION_OCTET_STREAM.as_ref()),
			)
			.body(full_body.clone())
			.unwrap();

		let RawBody(data) = RawBody::<1024>::from_request(request, Args::new())
			.await
			.unwrap();

		assert_eq!(&data, body);

		// ----------

		let request = Request::new(full_body.clone());

		let RawBody(data) = RawBody::<1024>::from_request(request, Args::new())
			.await
			.unwrap();

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

		let error = RawBody::<8>::from_request(request, Args::new())
			.await
			.unwrap_err();

		match error {
			RawBodyExtractorError::ContentTooLarge => {}
			error => panic!("unexpected error {}", error),
		}

		// ----------

		let request = Request::new(full_body.clone());

		let error = RawBody::<8>::from_request(request, Args::new())
			.await
			.unwrap_err();

		match error {
			RawBodyExtractorError::ContentTooLarge => {}
			error => panic!("unexpected error {}", error),
		}
	}
}
