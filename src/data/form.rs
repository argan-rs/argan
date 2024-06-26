//! Form data types.

// ----------

use argan_core::{
	body::HttpBody,
	request::{FromRequest, RequestHeadParts},
	response::{BoxedErrorResponse, IntoResponse, IntoResponseResult, Response},
	BoxedError,
};
use http::{header::CONTENT_TYPE, HeaderValue, StatusCode};
use serde::{de::DeserializeOwned, Serialize};

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

// --------------------------------------------------
// Form

pub(crate) const FORM_BODY_SIZE_LIMIT: usize = 2 * 1024 * 1024;

// ----------

/// Extractor and response type of the `application/x-www-form-urlencoded` data.
///
/// `Form` consumes the request body and deserializes it as type `T`. `T` must be a type
/// that implements [`serde::Deserialize`].
///
/// ```
/// use argan::data::form::Form;
/// use serde::Deserialize;
///
/// #[derive(Deserialize)]
/// struct Person {
///   first_name: String,
///   last_name: String,
///   age: u8,
/// }
///
/// async fn add_person(Form(person): Form<Person>) {
///   // ...
/// }
/// ```
///
/// By default, `Form` limits the body size to 2MiB. The body size limit can be changed by
/// specifying the SIZE_LIMIT const type parameter.
///
/// ```
/// use argan::data::form::Form;
/// use serde::Deserialize;
///
/// #[derive(Deserialize)]
/// struct SurveyData {
///   // ...
/// }
///
/// async fn save_survey_data(Form(survey_data): Form<SurveyData, { 512 * 1024 }>) {
///   // ...
/// }
/// ```
///
/// Usually, `GET` and `HEAD` requests carry the data in a query string. With these
/// requests, data can be obtained via [`RequestHead::query_params_as<T>`]. For this
/// to work `"query-params"` feature flag must be enabled.
///
///[`RequestHead::query_params_as<T>`]: crate::request::RequestHead
pub struct Form<T, const SIZE_LIMIT: usize = FORM_BODY_SIZE_LIMIT>(pub T);

impl<B, T, const SIZE_LIMIT: usize> FromRequest<B> for Form<T, SIZE_LIMIT>
where
	B: HttpBody + Send,
	B::Data: Send,
	B::Error: Into<BoxedError>,
	T: DeserializeOwned,
{
	type Error = FormError;

	async fn from_request(head_parts: &mut RequestHeadParts, body: B) -> Result<Self, Self::Error> {
		request_into_form_data(head_parts, body, SIZE_LIMIT)
			.await
			.map(Self)
	}
}

#[inline(always)]
pub(crate) async fn request_into_form_data<T, B>(
	head_parts: &RequestHeadParts,
	body: B,
	size_limit: usize,
) -> Result<T, FormError>
where
	B: HttpBody,
	B::Error: Into<BoxedError>,
	T: DeserializeOwned,
{
	use http_body_util::{BodyExt, LengthLimitError, Limited};

	use super::header::content_type;

	let content_type_str = content_type(head_parts)?;

	if content_type_str == mime::APPLICATION_WWW_FORM_URLENCODED {
		match Limited::new(body, size_limit).collect().await {
			Ok(body) => {
				Ok(serde_urlencoded::from_bytes::<T>(&body.to_bytes()).map_err(Into::<FormError>::into)?)
			}
			Err(error) => Err(
				error
					.downcast_ref::<LengthLimitError>()
					.map_or(FormError::BufferingFailure, |_| FormError::ContentTooLarge),
			),
		}
	} else {
		Err(FormError::UnsupportedMediaType)
	}
}

// ----------

impl<T> IntoResponseResult for Form<T>
where
	T: Serialize,
{
	fn into_response_result(self) -> Result<Response, BoxedErrorResponse> {
		let form_string = serde_urlencoded::to_string(self.0).map_err(Into::<FormError>::into)?;

		let mut response = form_string.into_response();
		response.headers_mut().insert(
			CONTENT_TYPE,
			HeaderValue::from_static(mime::APPLICATION_WWW_FORM_URLENCODED.as_ref()),
		);

		Ok(response)
	}
}

// ----------

data_extractor_error! {
	/// An error type that's returned on failure when extracting or serializing the `Form`.
	#[derive(Debug)]
	pub FormError {
		/// Returned when deserializing the body fails.
		#[error("{0}")]
		(DeserializationFailure(#[from] serde_urlencoded::de::Error)) [(_)];
		StatusCode::BAD_REQUEST;
		/// Returned when serializing the data fails.
		#[error("{0}")]
		(SerializationFailure(#[from] serde_urlencoded::ser::Error)) [(_)];
		StatusCode::INTERNAL_SERVER_ERROR;
	}
}

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

#[cfg(test)]
mod test {
	use serde::{Deserialize, Serialize};

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

	#[cfg(all(test, feature = "full"))]
	async fn form() {
		use argan_core::request::Request;
		use http::{header::CONTENT_TYPE, HeaderValue};

		let login = "login".to_string();
		let password = "password".to_string();

		let data = Data::new(login.clone(), password.clone());
		let form_data_string = serde_urlencoded::to_string(&data).unwrap();

		dbg!(&form_data_string);

		// ----------

		let (mut head_parts, body) = Request::builder()
			.header(
				CONTENT_TYPE,
				HeaderValue::from_static(mime::APPLICATION_WWW_FORM_URLENCODED.as_ref()),
			)
			.body(form_data_string)
			.unwrap()
			.into_parts();

		let Form(mut form_data) = Form::<Data>::from_request(&mut head_parts, body)
			.await
			.unwrap();

		assert_eq!(form_data.login, login.as_ref());
		assert_eq!(form_data.password, password.as_ref());

		// -----

		form_data.some_id = Some(1);
		let response = Form(form_data).into_response_result().unwrap();
		let form_body = response.into_body();

		// -----

		let (mut head_parts, body) = Request::builder()
			.header(
				CONTENT_TYPE,
				HeaderValue::from_static(mime::APPLICATION_WWW_FORM_URLENCODED.as_ref()),
			)
			.body(form_body)
			.unwrap()
			.into_parts();

		let Form(form_data) = Form::<Data>::from_request(&mut head_parts, body)
			.await
			.unwrap();

		assert_eq!(form_data.some_id, Some(1));
		assert_eq!(form_data.login, login.as_ref());
		assert_eq!(form_data.password, password.as_ref());
	}
}
