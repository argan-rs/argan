use http::{header::CONTENT_TYPE, HeaderValue, StatusCode};
use http_body_util::{BodyExt, LengthLimitError, Limited};
use serde::{de::DeserializeOwned, Serialize};
use serde_json::error::Category;

use crate::{
	body::HttpBody,
	common::BoxedError,
	handler::Args,
	header::{content_type, ToStrError},
	request::{FromRequest, Request},
	response::{IntoResponse, Response},
};

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
				Ok(body) => serde_json::from_slice::<T>(&body.to_bytes())
					.map(|value| Self(value))
					.map_err(Into::<JsonError>::into),
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

// --------------------------------------------------------------------------------
