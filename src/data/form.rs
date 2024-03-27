use std::io;

use futures_util::{Stream, StreamExt};
use http::HeaderMap;
use http_body_util::{BodyStream, Limited};
use mime::Mime;
use multer::parse_boundary;

use crate::{
	common::SCOPE_VALIDITY,
	handler::Args,
	response::{BoxedErrorResponse, IntoResponseResult},
	StdError,
};

use super::*;

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

// --------------------------------------------------
// Form

pub struct Form<T, const SIZE_LIMIT: usize = { 2 * 1024 * 1024 }>(pub T);

impl<'n, B, HandlerExt, T, const SIZE_LIMIT: usize> FromRequest<B, Args<'n, HandlerExt>, HandlerExt>
	for Form<T, SIZE_LIMIT>
where
	B: HttpBody + Send,
	B::Data: Send,
	B::Error: Into<BoxedError>,
	HandlerExt: Sync,
	T: DeserializeOwned,
{
	type Error = FormError;

	async fn from_request(
		request: Request<B>,
		_args: &mut Args<'n, HandlerExt>,
	) -> Result<Self, Self::Error> {
		let content_type_str = content_type(&request).map_err(Into::<FormError>::into)?;

		if content_type_str == mime::APPLICATION_WWW_FORM_URLENCODED {
			match Limited::new(request, SIZE_LIMIT).collect().await {
				Ok(body) => Ok(
					serde_urlencoded::from_bytes::<T>(&body.to_bytes())
						.map(|value| Self(value))
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
}

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

// --------------------------------------------------
// Multipart

pub struct Multipart<const SIZE_LIMIT: usize = { 8 * 1024 * 1024 }> {
	body_stream: BodyStream<Body>,
	boundary: String,
	some_constraints: Option<Constraints>,
}

impl<const SIZE_LIMIT: usize> Multipart<SIZE_LIMIT> {
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

		if let Some(constraints) = self.some_constraints.take() {
			let Constraints {
				inner: mut constraints,
				mut size_limit,
			} = constraints;

			size_limit = size_limit.whole_stream(SIZE_LIMIT as u64);
			constraints = constraints.size_limit(size_limit);

			Parts(multer::Multipart::with_constraints(
				data_stream,
				self.boundary,
				constraints,
			))
		} else {
			let size_limit = multer::SizeLimit::new().whole_stream(SIZE_LIMIT as u64);
			let constraints = multer::Constraints::new().size_limit(size_limit);

			Parts(multer::Multipart::with_constraints(
				data_stream,
				self.boundary,
				constraints,
			))
		}
	}
}

impl<'n, B, HandlerExt, const SIZE_LIMIT: usize> FromRequest<B, Args<'n, HandlerExt>, HandlerExt>
	for Multipart<SIZE_LIMIT>
where
	B: HttpBody<Data = Bytes> + Send + Sync + 'static,
	B::Error: Into<BoxedError>,
	HandlerExt: Sync,
{
	type Error = MultipartFormError;

	async fn from_request(
		request: Request<B>,
		_args: &mut Args<'n, HandlerExt>,
	) -> Result<Self, Self::Error> {
		let content_type_str = content_type(&request).map_err(Into::<MultipartFormError>::into)?;

		parse_boundary(content_type_str)
			.map(|boundary| {
				let incoming_body = Body::new(request.into_body());
				let body_stream = BodyStream::new(incoming_body);

				let multipart_form = Multipart {
					body_stream,
					boundary,
					some_constraints: None,
				};

				multipart_form
			})
			.map_err(Into::<MultipartFormError>::into)
	}
}

// ----------

pub struct Constraints {
	inner: multer::Constraints,
	size_limit: multer::SizeLimit,
}

impl Constraints {
	fn new() -> Self {
		Self {
			inner: multer::Constraints::new(),
			size_limit: multer::SizeLimit::new(),
		}
	}

	pub fn with_allowed_parts<S: Into<String>>(mut self, allowed_parts: Vec<S>) -> Self {
		self.inner = self.inner.allowed_fields(allowed_parts);

		self
	}

	pub fn with_size_limit(mut self, size_limit: SizeLimit) -> Self {
		self.size_limit = size_limit.0;

		self
	}
}

pub struct SizeLimit(multer::SizeLimit);

impl SizeLimit {
	pub fn new() -> Self {
		Self(multer::SizeLimit::new())
	}

	pub fn per_part(mut self, limit: usize) -> Self {
		self.0 = self.0.per_field(limit as u64);

		self
	}

	pub fn for_part<S: Into<String>>(mut self, part_name: S, limit: usize) -> Self {
		self.0 = self.0.for_field(part_name, limit as u64);

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

		let mut args = Args::new();

		// ----------

		let request = Request::builder()
			.header(
				CONTENT_TYPE,
				HeaderValue::from_static(mime::APPLICATION_WWW_FORM_URLENCODED.as_ref()),
			)
			.body(form_data_string)
			.unwrap();

		let Form(mut form_data) = Form::<Data>::from_request(request, &mut args)
			.await
			.unwrap();

		assert_eq!(form_data.login, login.as_ref());
		assert_eq!(form_data.password, password.as_ref());

		// -----

		form_data.some_id = Some(1);
		let response = Form(form_data).into_response_result().unwrap();
		let form_body = response.into_body();

		// -----

		let request = Request::builder()
			.header(
				CONTENT_TYPE,
				HeaderValue::from_static(mime::APPLICATION_WWW_FORM_URLENCODED.as_ref()),
			)
			.body(form_body)
			.unwrap();

		let Form(form_data) = Form::<Data>::from_request(request, &mut args)
			.await
			.unwrap();

		assert_eq!(form_data.some_id, Some(1));
		assert_eq!(form_data.login, login.as_ref());
		assert_eq!(form_data.password, password.as_ref());
	}
}
