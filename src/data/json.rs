use std::{str::FromStr, string::FromUtf8Error};

use http::{header::CONTENT_TYPE, HeaderValue, StatusCode};
use http_body_util::{BodyExt, LengthLimitError, Limited};
use serde::{de::DeserializeOwned, Serialize};
use serde_json::error::Category;

use crate::{
	handler::Args,
	request::{FromRequest, Request, SizeLimit},
	response::{BoxedErrorResponse, IntoResponse, IntoResponseResult, Response},
	routing::RoutingState,
};

use super::*;

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

// --------------------------------------------------

pub(crate) const JSON_BODY_SIZE_LIMIT: usize = { 2 * 1024 * 1024 };

// ----------

#[inline(always)]
pub(crate) async fn request_into_json_data<T, B>(
	request: Request<B>,
	size_limit: usize,
) -> Result<T, JsonError>
where
	B: HttpBody,
	B::Error: Into<BoxedError>,
	T: DeserializeOwned,
{
	let content_type = content_type(&request)?;

	if content_type == mime::APPLICATION_JSON {
		match Limited::new(request, size_limit).collect().await {
			Ok(body) => serde_json::from_slice::<T>(&body.to_bytes())
				.map(|value| value)
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

// --------------------------------------------------
// Json

pub struct Json<T, const SIZE_LIMIT: usize = JSON_BODY_SIZE_LIMIT>(pub T);

impl<B, T, const SIZE_LIMIT: usize> FromRequest<B> for Json<T, SIZE_LIMIT>
where
	B: HttpBody + Send,
	B::Data: Send,
	B::Error: Into<BoxedError>,
	T: DeserializeOwned,
{
	type Error = JsonError;

	async fn from_request(request: Request<B>) -> Result<Self, Self::Error> {
		request_into_json_data::<T, B>(request, SIZE_LIMIT)
			.await
			.map(Self)
	}
}

impl<T> IntoResponseResult for Json<T>
where
	T: Serialize,
{
	fn into_response_result(self) -> Result<Response, BoxedErrorResponse> {
		let json_string = serde_json::to_string(&self.0).map_err(Into::<JsonError>::into)?;

		let mut response = json_string.into_response();
		response.headers_mut().insert(
			CONTENT_TYPE,
			HeaderValue::from_static(mime::APPLICATION_JSON.as_ref()),
		);

		Ok(response)
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
// --------------------------------------------------------------------------------

#[cfg(test)]
mod test {
	use serde::Deserialize;

	use super::*;

	// --------------------------------------------------------------------------------
	// --------------------------------------------------------------------------------

	#[derive(Debug, Serialize, Deserialize)]
	struct Data {
		some_id: Option<u32>,
		login: String,
		password: String,
	}

	impl Data {
		fn new(login: String, password: String) -> Self {
			Self {
				some_id: None,
				login,
				password,
			}
		}
	}

	// -------------------------

	#[tokio::test]
	async fn json() {
		let login = "login".to_string();
		let password = "password".to_string();

		let data = Data::new(login.clone(), password.clone());
		let json_data_string = serde_json::to_string(&data).unwrap();

		dbg!(&json_data_string);

		// ----------

		let request = Request::builder()
			.header(
				CONTENT_TYPE,
				HeaderValue::from_static(mime::APPLICATION_JSON.as_ref()),
			)
			.body(json_data_string)
			.unwrap();

		let Json(mut json_data) = Json::<Data>::from_request(request).await.unwrap();

		assert_eq!(json_data.login, login.as_ref());
		assert_eq!(json_data.password, password.as_ref());

		// -----

		json_data.some_id = Some(1);
		let response = Json(json_data).into_response_result().unwrap();
		let json_body = response.into_body();

		// -----

		let request = Request::builder()
			.header(
				CONTENT_TYPE,
				HeaderValue::from_static(mime::APPLICATION_JSON.as_ref()),
			)
			.body(json_body)
			.unwrap();

		let Json(json_data) = Json::<Data>::from_request(request).await.unwrap();

		assert_eq!(json_data.some_id, Some(1));
		assert_eq!(json_data.login, login.as_ref());
		assert_eq!(json_data.password, password.as_ref());
	}
}
