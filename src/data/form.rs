//! Form data types.

// ----------

use std::io;

use argan_core::request::RequestHeadParts;
use futures_util::{Stream, StreamExt};
use http::HeaderMap;
use http_body_util::{BodyStream, Limited};
use mime::Mime;

#[cfg(feature = "multipart-form")]
use multer::parse_boundary;

use crate::{
	common::SCOPE_VALIDITY,
	response::{BoxedErrorResponse, IntoResponseResult},
	routing::RoutingState,
	StdError,
};

use super::*;

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

// --------------------------------------------------
// Form

#[cfg(feature = "form")]
pub(crate) const FORM_BODY_SIZE_LIMIT: usize = { 2 * 1024 * 1024 };

// ----------

/// Extractor and response type of the `application/x-www-form-urlencoded` data.
///
/// `Form` consumes the request body and deserializes it as type `T`. `T` must be a type
/// that implements [`serde::Deserialize`].
///
/// ```
///	use argan::data::form::Form;
/// use serde::Deserialize;
///
/// #[derive(Deserialize)]
/// struct Person {
///		first_name: String,
///		last_name: String,
///		age: u8,
/// }
///
/// async fn add_person(Form(person): Form<Person>) {
///		// ...
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
///		// ...
/// }
///
/// async fn save_survey_data(Form(survey_data): Form<SurveyData, { 512 * 1024 }>) {
///		// ...
/// }
/// ```
///
/// Usually, `GET` and `HEAD` requests carry the data in a query string. With these
/// requests, data can be obtained via [`RequestHead::query_params_as<T>`].
#[cfg(feature = "form")]
pub struct Form<T, const SIZE_LIMIT: usize = FORM_BODY_SIZE_LIMIT>(pub T);

#[cfg(feature = "form")]
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

#[cfg(feature = "form")]
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
	let content_type_str = content_type(head_parts)?;

	if content_type_str == mime::APPLICATION_WWW_FORM_URLENCODED {
		match Limited::new(body, size_limit).collect().await {
			Ok(body) => Ok(
				serde_urlencoded::from_bytes::<T>(&body.to_bytes())
					.map(|value| value)
					.map_err(Into::<FormError>::into)?,
			),
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

#[cfg(feature = "form")]
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

#[cfg(feature = "form")]
data_extractor_error! {
	/// An error type returned on failures when extracting or serializing the `Form`.
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
// --------------------------------------------------------------------------------

// --------------------------------------------------
// MultipartForm

#[cfg(feature = "multipart-form")]
const MULTIPART_FORM_BODY_SIZE_LIMIT: usize = { 8 * 1024 * 1024 };

// ----------

/// Extractor of `multipart/form-data`.
///
/// ```
/// use argan::data::form::{MultipartForm, Constraints, MultipartFormError};
///
/// async fn upload_handler(multipart_form: MultipartForm) -> Result<(), MultipartFormError> {
///		let constraints = Constraints::new().with_allowed_parts(vec!["name", "picture"]);
///		let mut parts = multipart_form.with_constraints(constraints).into_parts();
///
///		while let Some(part) = parts.next().await? {
///			// ...
///		}
///
///		Ok(())
/// }
/// ```
#[cfg(feature = "multipart-form")]
pub struct MultipartForm<B = Body> {
	body_stream: BodyStream<B>,
	boundary: String,
	some_constraints: Option<Constraints>,
}

#[cfg(feature = "multipart-form")]
impl<B> MultipartForm<B>
where
	B: HttpBody<Data = Bytes> + Send + 'static,
	B::Error: Into<BoxedError> + 'static,
{
	/// Sets the constraints on the multipart form.
	///
	/// By default, a full body size limit is set, which defaults to 8MiB.
	pub fn with_constraints(mut self, constraints: Constraints) -> Self {
		self.some_constraints = Some(constraints);

		self
	}

	/// Converts the `MultipartForm` into an *"async iterator"* over the parts.
	pub fn into_parts(mut self) -> Parts {
		let data_stream = self.body_stream.map(|result| {
			match result {
				Ok(frame) => {
					match frame.into_data() {
						Ok(data) => Ok(data),
						Err(_) => Ok(Bytes::new()), // ??? Trailers are being ignored.
					}
				}
				Err(error) => Err(error),
			}
		});

		let constraints = if let Some(constraints) = self.some_constraints.take() {
			let Constraints {
				inner: mut constraints,
				body_size_limit,
				some_part_size_limit,
				some_size_limits_for_parts,
			} = constraints;

			let mut size_limit = multer::SizeLimit::new();

			size_limit = size_limit.whole_stream(body_size_limit);

			if let Some(part_size_limit) = some_part_size_limit {
				size_limit = size_limit.per_field(part_size_limit);
			}

			if let Some(size_limits_for_parts) = some_size_limits_for_parts {
				for (part_name, limit) in size_limits_for_parts {
					size_limit = size_limit.for_field(part_name, limit);
				}
			}

			constraints.size_limit(size_limit)
		} else {
			let size_limit = multer::SizeLimit::new().whole_stream(MULTIPART_FORM_BODY_SIZE_LIMIT as u64);

			multer::Constraints::new().size_limit(size_limit)
		};

		Parts(multer::Multipart::with_constraints(
			data_stream,
			self.boundary,
			constraints,
		))
	}
}

#[cfg(feature = "multipart-form")]
impl<B> FromRequest<B> for MultipartForm<B>
where
	B: HttpBody<Data = Bytes> + Send + Sync + 'static,
	B::Error: Into<BoxedError>,
{
	type Error = MultipartFormError;

	async fn from_request(head_parts: &mut RequestHeadParts, body: B) -> Result<Self, Self::Error> {
		request_into_multipart_form(head_parts, body)
	}
}

#[cfg(feature = "multipart-form")]
#[inline(always)]
pub(crate) fn request_into_multipart_form<B>(
	head_parts: &RequestHeadParts,
	body: B,
) -> Result<MultipartForm<B>, MultipartFormError>
where
	B: HttpBody<Data = Bytes> + Send + Sync + 'static,
	B::Error: Into<BoxedError>,
{
	let content_type_str = content_type(head_parts)?;

	parse_boundary(content_type_str)
		.map(|boundary| {
			let body_stream = BodyStream::new(body);

			let multipart_form = MultipartForm {
				body_stream,
				boundary,
				some_constraints: None,
			};

			multipart_form
		})
		.map_err(Into::<MultipartFormError>::into)
}

// ----------

/// Constraints to limit the extraction of parts.
#[cfg(feature = "multipart-form")]
pub struct Constraints {
	inner: multer::Constraints,
	body_size_limit: u64,
	some_part_size_limit: Option<u64>,
	some_size_limits_for_parts: Option<Vec<(String, u64)>>,
}

#[cfg(feature = "multipart-form")]
impl Constraints {
	/// Creates a new `Constraints` with only a body size limit, which defaults to 8MiB.
	pub fn new() -> Self {
		Self {
			inner: multer::Constraints::new(),
			body_size_limit: MULTIPART_FORM_BODY_SIZE_LIMIT as u64,
			some_part_size_limit: None,
			some_size_limits_for_parts: None,
		}
	}

	/// Limits the multipart form only to the specified parts.
	pub fn with_allowed_parts<S: Into<String>>(mut self, allowed_parts: Vec<S>) -> Self {
		self.inner = self.inner.allowed_fields(allowed_parts);

		self
	}

	/// Sets the whole body size limit on the multipart form.
	pub fn with_body_size_limit(mut self, size_limit: u64) -> Self {
		self.body_size_limit = size_limit;

		self
	}

	/// Sets the maximum size limit for each part of the multipart form.
	pub fn with_part_size_limit(mut self, size_limit: u64) -> Self {
		self.some_part_size_limit = Some(size_limit);

		self
	}

	/// Sets size limits on the specified parts.
	pub fn with_size_limits_for_parts(mut self, size_limits_for_parts: Vec<(String, u64)>) -> Self {
		self.some_size_limits_for_parts = Some(size_limits_for_parts);

		self
	}
}

// ----------

/// An *"async iterator"* over the parts of the multipart form.
#[cfg(feature = "multipart-form")]
pub struct Parts(multer::Multipart<'static>);

#[cfg(feature = "multipart-form")]
impl Parts {
	/// Returns the next part of the multipart form.
	pub async fn next<'p>(&'p mut self) -> Result<Option<Part<'p>>, MultipartFormError> {
		self
			.0
			.next_field()
			.await
			.map(|some_field| {
				some_field.map(|field| Part {
					inner: field,
					_lifetime_mark: PhantomData,
				})
			})
			.map_err(|error| error.into())
	}
}

/// Single part of the multipart form.
#[cfg(feature = "multipart-form")]
pub struct Part<'p> {
	inner: multer::Field<'static>,
	_lifetime_mark: PhantomData<&'p mut Parts>,
}

#[cfg(feature = "multipart-form")]
impl<'p> Part<'p> {
	/// Returns the index of the part in the multipart form.
	pub fn index(&self) -> usize {
		self.inner.index()
	}

	/// Returns the value of the `Content-Disposition` `name` attribute.
	pub fn name(&self) -> Option<&str> {
		self.inner.name()
	}

	/// Returns the content type of the part.
	pub fn content_type(&self) -> Option<&Mime> {
		self.inner.content_type()
	}

	/// Returns the headers of the part.
	pub fn headers(&self) -> &HeaderMap {
		self.inner.headers()
	}

	/// Returns the value of the `Content-Disposition` `filename` attribute.
	pub fn file_name(&self) -> Option<&str> {
		self.inner.file_name()
	}

	/// Returns the full payload of the part.
	pub async fn bytes(self) -> Result<Bytes, MultipartFormError> {
		self.inner.bytes().await.map_err(|error| error.into())
	}

	/// Returns the available chunk of the part's payload.
	pub async fn chunk(&mut self) -> Result<Option<Bytes>, MultipartFormError> {
		self.inner.chunk().await.map_err(|error| error.into())
	}

	/// Returns the full payload of the part as text.
	pub async fn text(self) -> Result<String, MultipartFormError> {
		self.inner.text().await.map_err(|error| error.into())
	}

	/// Tries to convert the full payload to a text with the given charset,
	/// returning it on success.
	pub async fn text_with_charset(
		self,
		default_charset: &str,
	) -> Result<String, MultipartFormError> {
		self
			.inner
			.text_with_charset(default_charset)
			.await
			.map_err(|error| error.into())
	}

	/// Tries to deserialize the part's payload as JSON data.
	#[cfg(feature = "json")]
	pub async fn json<T: DeserializeOwned>(self) -> Result<T, MultipartFormError> {
		self.inner.json().await.map_err(|error| error.into())
	}
}

// ----------

#[cfg(feature = "multipart-form")]
data_extractor_error! {
	/// An error type returned on failures when extracting the `MultipartForm`.
	#[derive(Debug)]
	pub MultipartFormError {
		/// Returned when the form is constrained to certain parts and an unknown part is detected.
		#[error("unkown part {part_name}")]
		(UnknownPart { part_name: String }) [{..}]; StatusCode::BAD_REQUEST;
		/// Returned when collecting some part's data has failed.
		#[error("incomplete part {part_name} data")]
		(IncompletePartData { part_name: String }) [{..}]; StatusCode::BAD_REQUEST;
		/// Returned when collecting the part's headers has failed.
		#[error("incomplete part headers")]
		(IncompletePartHeaders) StatusCode::BAD_REQUEST;
		/// Returned on failure when reading the part's headers.
		#[error("part headers read failure")]
		(PartHeadersReadFailure(httparse::Error)) [(_)]; StatusCode::BAD_REQUEST;
		/// Returned when invalid part header name is detected.
		#[error("part header name {header_name} decoding failure")]
		(InvalidPartHeaderName { header_name: String, cause: Box<dyn StdError + Send + Sync> })  [{..}];
			StatusCode::BAD_REQUEST;
		/// Returned when invalid part header value is detected.
		#[error("part header value decoding failure")]
		(InvalidPartHeaderValue {
			value: Vec<u8>,
			cause: Box<dyn StdError + Send + Sync>,
		}) [{..}]; StatusCode::BAD_REQUEST;
		/// Returned when some part's `Content-Type` is invalid.
		#[error("part Content-Type decoding failure")]
		(InvlaidPartContentType(mime::FromStrError)) [(_)]; StatusCode::BAD_REQUEST;
		/// Returned when the multipart form body is incomplete.
		#[error("incomplete multipart form body")]
		(IncompleteBody) StatusCode::BAD_REQUEST;
		/// Returned when some part's size overflows its limit.
		#[error("part size limit overflow")]
		(PartSizeLimitOverflow { part_name: String }) [{..}]; StatusCode::PAYLOAD_TOO_LARGE;
		/// Returned on failure when reading the multipart form body.
		#[error("failure on reading the multipart form body")]
		(BodyReadFailure(Box<dyn StdError + Send + Sync>)) [(_)]; StatusCode::BAD_REQUEST;
		/// Returned on failure when locking the internal shared state.
		#[error("internal state lock failure")]
		(InternalStateLockFailure) StatusCode::INTERNAL_SERVER_ERROR;
		/// Returned when multipart form `Content-Type` has no boundary attribute.
		#[error("no boundary")]
		(NoBoundary) StatusCode::BAD_REQUEST;
		/// Returned on syntax error when deserializing the part's payload as JSON data.
		#[cfg(feature = "json")]
		#[error("invlaid JSON syntax in line {line}, column {column}")]
		(InvalidJsonSyntax { line: usize, column: usize}) [{..}]; StatusCode::BAD_REQUEST;
		/// Returned on semantically incorrect data when deserializing the part's payload as JSON.
		#[cfg(feature = "json")]
		#[error("invalid JSON semantics in line {line}, column {column}")]
		(InvalidJsonData { line: usize, column: usize}) [{..}]; StatusCode::UNPROCESSABLE_ENTITY;
		/// Returned on read failure when deserializing the part's payload as JSON.
		#[cfg(feature = "json")]
		#[error("JSON I/O stream failure")]
		(JsonIoFailure(io::ErrorKind)) [{..}]; StatusCode::INTERNAL_SERVER_ERROR;
		/// Returned on unexpected *end of file* when deserializing the part's payload as JSON.
		#[cfg(feature = "json")]
		#[error("JSON unexpected end of file")]
		(JsonUnexpectedEoF) StatusCode::BAD_REQUEST;
		/// Returned on unknown failure.
		#[error("unknown failure")]
		(UnknownFailure) StatusCode::INTERNAL_SERVER_ERROR;
	}
}

#[cfg(feature = "multipart-form")]
impl From<multer::Error> for MultipartFormError {
	fn from(error: multer::Error) -> Self {
		use multer::Error::*;
		match error {
			UnknownField { field_name } => Self::UnknownPart {
				part_name: field_name.unwrap_or(String::new()),
			},
			IncompleteFieldData { field_name } => Self::IncompletePartData {
				part_name: field_name.unwrap_or(String::new()),
			},
			IncompleteHeaders => Self::IncompletePartHeaders,
			ReadHeaderFailed(parse_error) => Self::PartHeadersReadFailure(parse_error),
			DecodeHeaderName { name, cause } => Self::InvalidPartHeaderName {
				header_name: name,
				cause,
			},
			DecodeHeaderValue { value, cause } => Self::InvalidPartHeaderValue { value, cause },
			IncompleteStream => Self::IncompleteBody,
			FieldSizeExceeded { field_name, .. } => Self::PartSizeLimitOverflow {
				part_name: field_name.unwrap_or(String::new()),
			},
			StreamSizeExceeded { .. } => Self::ContentTooLarge,
			StreamReadFailed(error) => Self::BodyReadFailure(error),
			LockFailure => Self::InternalStateLockFailure,
			NoMultipart => Self::UnsupportedMediaType,
			DecodeContentType(error) => Self::InvlaidPartContentType(error),
			NoBoundary => Self::NoBoundary,
			#[cfg(feature = "json")]
			DecodeJson(error) => match error.classify() {
				Category::Io => Self::JsonIoFailure(error.io_error_kind().expect(SCOPE_VALIDITY)),
				Category::Syntax => Self::InvalidJsonSyntax {
					line: error.line(),
					column: error.column(),
				},
				Category::Data => Self::InvalidJsonData {
					line: error.line(),
					column: error.column(),
				},
				Category::Eof => Self::JsonUnexpectedEoF,
			},
			_ => Self::UnknownFailure,
		}
	}
}

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

#[cfg(all(test, feature = "full"))]
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

	#[cfg(all(test, feature = "full"))]
	async fn form() {
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
