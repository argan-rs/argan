use std::io;

use argan_core::request::RequestHeadParts;
use futures_util::{Stream, StreamExt};
use http::HeaderMap;
use http_body_util::{BodyStream, Limited};
use mime::Mime;
use multer::parse_boundary;

use crate::{
	common::SCOPE_VALIDITY,
	handler::Args,
	response::{BoxedErrorResponse, IntoResponseResult},
	routing::RoutingState,
	StdError,
};

use super::*;

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

// --------------------------------------------------
// Form

pub(crate) const FORM_BODY_SIZE_LIMIT: usize = { 2 * 1024 * 1024 };

// ----------

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
	#[derive(Debug)]
	pub FormError {
		#[error("{0}")]
		(DeserializationFailure(#[from] serde_urlencoded::de::Error)) [(_)];
		StatusCode::BAD_REQUEST;
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

const MULTIPART_FORM_BODY_SIZE_LIMIT: usize = { 8 * 1024 * 1024 };

// ----------

pub struct MultipartForm<B> {
	body_stream: BodyStream<B>,
	boundary: String,
	some_constraints: Option<Constraints>,
}

impl<B> MultipartForm<B>
where
	B: HttpBody<Data = Bytes> + Send + 'static,
	B::Error: Into<BoxedError> + 'static,
{
	fn with_constraints(mut self, constraints: Constraints) -> Self {
		self.some_constraints = Some(constraints);

		self
	}

	fn into_parts(mut self) -> Parts {
		let data_stream = self.body_stream.map(|result| {
			match result {
				Ok(frame) => {
					match frame.into_data() {
						Ok(data) => Ok(data),
						Err(_) => Ok(Bytes::new()), // ??? Trailers are being ignored for now.
					}
				}
				Err(error) => Err(error),
			}
		});

		let constraints = if let Some(constraints) = self.some_constraints.take() {
			let Constraints {
				inner: mut constraints,
				some_body_size_limit,
				some_part_size_limit,
				some_size_limits_for_parts,
			} = constraints;

			let mut size_limit = multer::SizeLimit::new();

			if let Some(body_size_limit) = some_body_size_limit {
				size_limit = size_limit.whole_stream(body_size_limit);
			} else {
				size_limit = size_limit.whole_stream(MULTIPART_FORM_BODY_SIZE_LIMIT as u64);
			}

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

pub struct Constraints {
	inner: multer::Constraints,
	some_body_size_limit: Option<u64>,
	some_part_size_limit: Option<u64>,
	some_size_limits_for_parts: Option<Vec<(String, u64)>>,
}

impl Constraints {
	fn new() -> Self {
		Self {
			inner: multer::Constraints::new(),
			some_body_size_limit: None,
			some_part_size_limit: None,
			some_size_limits_for_parts: None,
		}
	}

	pub fn with_allowed_parts<S: Into<String>>(mut self, allowed_parts: Vec<S>) -> Self {
		self.inner = self.inner.allowed_fields(allowed_parts);

		self
	}

	pub fn with_body_size_limit(mut self, size_limit: u64) -> Self {
		self.some_body_size_limit = Some(size_limit);

		self
	}

	pub fn with_part_size_limit(mut self, size_limit: u64) -> Self {
		self.some_part_size_limit = Some(size_limit);

		self
	}

	pub fn with_size_limits_for_parts(mut self, size_limits_for_parts: Vec<(String, u64)>) -> Self {
		self.some_size_limits_for_parts = Some(size_limits_for_parts);

		self
	}
}

// ----------

pub struct Parts(multer::Multipart<'static>);

impl Parts {
	pub async fn next(&mut self) -> Result<Option<Part>, MultipartFormError> {
		self
			.0
			.next_field()
			.await
			.map(|some_field| some_field.map(|field| Part(field)))
			.map_err(|error| error.into())
	}

	pub async fn next_with_index(&mut self) -> Result<Option<(usize, Part)>, MultipartFormError> {
		self
			.0
			.next_field_with_idx()
			.await
			.map(|some_field_with_index| some_field_with_index.map(|(index, field)| (index, Part(field))))
			.map_err(|error| error.into())
	}
}

impl Stream for Parts {
	type Item = Result<Option<Part>, MultipartFormError>;

	fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
		pin!(self.0.next_field()).poll(cx).map(|result| {
			Some(
				result
					.map(|some_filed| some_filed.map(|field| Part(field)))
					.map_err(|error| error.into()),
			)
		})
	}
}

pub struct Part(multer::Field<'static>);

impl Part {
	pub fn index(&self) -> usize {
		self.0.index()
	}

	pub fn name(&self) -> Option<&str> {
		self.0.name()
	}

	pub fn content_type(&self) -> Option<&Mime> {
		self.0.content_type()
	}

	pub fn headers(&self) -> &HeaderMap {
		self.0.headers()
	}

	pub fn file_name(&self) -> Option<&str> {
		self.0.file_name()
	}

	pub async fn bytes(self) -> Result<Bytes, MultipartFormError> {
		self.0.bytes().await.map_err(|error| error.into())
	}

	pub async fn chunk(&mut self) -> Result<Option<Bytes>, MultipartFormError> {
		self.0.chunk().await.map_err(|error| error.into())
	}

	pub async fn text(self) -> Result<String, MultipartFormError> {
		self.0.text().await.map_err(|error| error.into())
	}

	pub async fn text_with_charset(
		self,
		default_charset: &str,
	) -> Result<String, MultipartFormError> {
		self
			.0
			.text_with_charset(default_charset)
			.await
			.map_err(|error| error.into())
	}

	pub async fn json<T: DeserializeOwned>(self) -> Result<T, MultipartFormError> {
		self.0.json().await.map_err(|error| error.into())
	}
}

// ----------

data_extractor_error! {
	#[derive(Debug)]
	pub MultipartFormError {
		#[error("unkown part {part_name}")]
		(UnknownPart { part_name: String }) [{..}]; StatusCode::BAD_REQUEST;
		#[error("incomplete part {part_name} data")]
		(IncompletePartData { part_name: String }) [{..}]; StatusCode::BAD_REQUEST;
		#[error("incomplete part headers")]
		(IncompletePartHeaders) StatusCode::BAD_REQUEST;
		#[error("part headers read failure")]
		(PartHeadersReadFailure(httparse::Error)) [(_)]; StatusCode::BAD_REQUEST;
		#[error("part header name {header_name} decoding failure")]
		(InvalidPartHeaderName { header_name: String, cause: Box<dyn StdError + Send + Sync> })  [{..}];
			StatusCode::BAD_REQUEST;
		#[error("part header value decoding failure")]
		(InvalidPartHeaderValue {
			value: Vec<u8>,
			cause: Box<dyn StdError + Send + Sync>,
		}) [{..}]; StatusCode::BAD_REQUEST;
		#[error("incomplete stream")]
		(IncompleteStream) StatusCode::BAD_REQUEST;
		#[error("part size limit overflow")]
		(PartSizeLimitOverflow { part_name: String }) [{..}]; StatusCode::PAYLOAD_TOO_LARGE;
		#[error("stream read failure")]
		(StreamReadFailure(Box<dyn StdError + Send + Sync>)) [(_)]; StatusCode::BAD_REQUEST;
		#[error("internal state lock failure")]
		(InternalStateLockFailure) StatusCode::INTERNAL_SERVER_ERROR;
		#[error("part Content-Type decoding failure")]
		(InvlaidPartContentType(mime::FromStrError)) [(_)]; StatusCode::BAD_REQUEST;
		#[error("no boundary")]
		(NoBoundary) StatusCode::BAD_REQUEST;
		#[error("invlaid JSON syntax in line {line}, column {column}")]
		(InvalidJsonSyntax { line: usize, column: usize}) [{..}]; StatusCode::BAD_REQUEST;
		#[error("invalid JSON semantics in line {line}, column {column}")]
		(InvalidJsonData { line: usize, column: usize}) [{..}]; StatusCode::UNPROCESSABLE_ENTITY;
		#[error("JSON I/O stream failure")]
		(JsonIoFailure(io::ErrorKind)) [{..}]; StatusCode::INTERNAL_SERVER_ERROR;
		#[error("JSON unexpected end of file")]
		(JsonUnexpectedEoF) StatusCode::BAD_REQUEST;
		#[error("unknown failure")]
		(UnknownFailure) StatusCode::INTERNAL_SERVER_ERROR;
	}
}

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
			IncompleteStream => Self::IncompleteStream,
			FieldSizeExceeded { field_name, .. } => Self::PartSizeLimitOverflow {
				part_name: field_name.unwrap_or(String::new()),
			},
			StreamSizeExceeded { .. } => Self::ContentTooLarge,
			StreamReadFailed(error) => Self::StreamReadFailure(error),
			LockFailure => Self::InternalStateLockFailure,
			NoMultipart => Self::UnsupportedMediaType,
			DecodeContentType(error) => Self::InvlaidPartContentType(error),
			NoBoundary => Self::NoBoundary,
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
