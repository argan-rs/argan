use futures_util::{Stream, StreamExt};
use http_body_util::{BodyStream, Limited};
use mime::Mime;
use multer::parse_boundary;

use crate::{body::IncomingBody, request::content_type, utils::BoxedError};

use super::*;

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

// --------------------------------------------------
// Form

pub struct Form<T, const SIZE_LIMIT: usize = { 2 * 1024 * 1024 }>(pub T);

impl<B, T, const SIZE_LIMIT: usize> FromRequest<B> for Form<T, SIZE_LIMIT>
where
	B: Body + Send,
	B::Data: Send,
	B::Error: Into<BoxedError>,
	T: DeserializeOwned,
{
	type Error = FormError;

	async fn from_request(request: Request<B>) -> Result<Self, Self::Error> {
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

impl<T> IntoResponse for Form<T>
where
	T: Serialize,
{
	fn into_response(self) -> Response {
		let form_string =
			match serde_urlencoded::to_string(self.0).map_err(|_| FormError::SerializationFailure) {
				Ok(form_string) => form_string,
				Err(error) => return error.into_response(),
			};

		let mut response = form_string.into_response();
		response.headers_mut().insert(
			CONTENT_TYPE,
			HeaderValue::from_static(mime::APPLICATION_WWW_FORM_URLENCODED.as_ref()),
		);

		response
	}
}

// ----------

#[non_exhaustive]
#[derive(Debug, ImplError)]
pub enum FormError {
	#[error(transparent)]
	MissingContentType(HeaderError),
	#[error(transparent)]
	InvalidContentType(HeaderError),
	#[error("unsupported media type")]
	UnsupportedMediaType,
	#[error("content too large")]
	ContentTooLarge,
	#[error("buffering failure")]
	BufferingFailure,
	#[error(transparent)]
	DeserializationFailure(#[from] serde_urlencoded::de::Error),
	#[error("serialization failure")]
	SerializationFailure,
}

impl From<HeaderError> for FormError {
	fn from(header_error: HeaderError) -> Self {
		match header_error {
			HeaderError::MissingHeader(_) => FormError::MissingContentType(header_error),
			HeaderError::InvalidValue(_) => FormError::InvalidContentType(header_error),
		}
	}
}

impl IntoResponse for FormError {
	fn into_response(self) -> Response {
		use FormError::*;

		match self {
			MissingContentType(_) | InvalidContentType(_) => StatusCode::BAD_REQUEST.into_response(),
			UnsupportedMediaType => StatusCode::UNSUPPORTED_MEDIA_TYPE.into_response(),
			ContentTooLarge => StatusCode::PAYLOAD_TOO_LARGE.into_response(),
			BufferingFailure | SerializationFailure => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
			DeserializationFailure(_) => StatusCode::BAD_REQUEST.into_response(),
		}
	}
}

// --------------------------------------------------
// Multipart

struct Multipart<const SIZE_LIMIT: usize = { 8 * 1024 * 1024 }> {
	body_stream: BodyStream<Limited<IncomingBody>>,
	boundary: String,
	constraints: Option<Constraints>,
}

impl<const SIZE_LIMIT: usize> Multipart<SIZE_LIMIT> {
	fn with_constraints(mut self, constraints: Constraints) -> Self {
		self.constraints = Some(constraints);

		self
	}

	fn into_parts(mut self) -> Parts {
		let data_stream = self.body_stream.map(|result| {
			match result {
				Ok(frame) => {
					match frame.into_data() {
						Ok(data) => Ok(data),
						Err(_) => Ok(Bytes::new()), // ???
					}
				}
				Err(error) => Err(error),
			}
		});

		if let Some(constraints) = self.constraints.take() {
			Parts(multer::Multipart::with_constraints(
				data_stream,
				self.boundary,
				constraints.0,
			))
		} else {
			Parts(multer::Multipart::new(data_stream, self.boundary))
		}
	}
}

impl<B, const SIZE_LIMIT: usize> FromRequest<B> for Multipart<SIZE_LIMIT>
where
	B: Body + Send + Sync + 'static,
	B::Error: Into<BoxedError>,
{
	type Error = StatusCode; // TODO.

	async fn from_request(request: Request<B>) -> Result<Self, Self::Error> {
		let content_type_str = content_type(&request).map_err(|_| StatusCode::BAD_REQUEST)?;

		if let Ok(boundary) = parse_boundary(content_type_str) {
			let body = request.into_body();
			let limited_incoming_body = Limited::new(IncomingBody::new(body), SIZE_LIMIT);

			let body_stream = BodyStream::new(limited_incoming_body);
			let multipart_form = Multipart {
				body_stream,
				boundary,
				constraints: None,
			};

			Ok(multipart_form)
		} else {
			Err(StatusCode::UNSUPPORTED_MEDIA_TYPE)
		}
	}
}

// ----------

struct Constraints(multer::Constraints);

impl Constraints {
	fn new() -> Self {
		Self(multer::Constraints::new())
	}

	fn with_allowed_parts<S: Into<String>>(mut self, allowed_parts: Vec<S>) -> Self {
		self.0 = self.0.allowed_fields(allowed_parts);

		self
	}

	fn with_size_limit(mut self, size_limit: SizeLimit) -> Self {
		self.0 = self.0.size_limit(size_limit.0);

		self
	}
}

struct SizeLimit(multer::SizeLimit);

impl SizeLimit {
	fn new() -> Self {
		Self(multer::SizeLimit::new())
	}

	fn per_part(mut self, limit: usize) -> Self {
		self.0 = self.0.per_field(limit as u64);

		self
	}

	fn for_part<S: Into<String>>(mut self, part_name: S, limit: usize) -> Self {
		self.0 = self.0.for_field(part_name, limit as u64);

		self
	}
}

// ----------

pub struct Parts(multer::Multipart<'static>);

impl Parts {
	pub async fn next(&mut self) -> Result<Option<Part>, Error> {
		self
			.0
			.next_field()
			.await
			.map(|some_field| some_field.map(|field| Part(field)))
			.map_err(|error| Error(error))
	}

	pub async fn next_with_index(&mut self) -> Result<Option<(usize, Part)>, Error> {
		self
			.0
			.next_field_with_idx()
			.await
			.map(|some_field_with_index| some_field_with_index.map(|(index, field)| (index, Part(field))))
			.map_err(|error| Error(error))
	}
}

impl Stream for Parts {
	type Item = Result<Option<Part>, Error>;

	fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
		pin!(self.0.next_field()).poll(cx).map(|result| {
			Some(
				result
					.map(|some_filed| some_filed.map(|field| Part(field)))
					.map_err(|error| Error(error)),
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

	pub async fn bytes(self) -> Result<Bytes, Error> {
		self.0.bytes().await.map_err(|error| Error(error))
	}

	pub async fn chunk(&mut self) -> Result<Option<Bytes>, Error> {
		self.0.chunk().await.map_err(|error| Error(error))
	}

	pub async fn text(self) -> Result<String, Error> {
		self.0.text().await.map_err(|error| Error(error))
	}

	pub async fn text_with_charset(self, default_charset: &str) -> Result<String, Error> {
		self
			.0
			.text_with_charset(default_charset)
			.await
			.map_err(|error| Error(error))
	}

	pub async fn json<T: DeserializeOwned>(self) -> Result<T, Error> {
		self.0.json().await.map_err(|error| Error(error))
	}
}

// ----------

pub struct Error(multer::Error); // TODO: Implement std Error, Display, Debug, IntoResponse, Eq.

// --------------------------------------------------------------------------------
