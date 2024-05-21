//! Low-level file streaming.

// ----------

use std::{
	fs::File,
	io::{Error as IoError, ErrorKind, Read, Seek, SeekFrom},
	path::Path,
	pin::Pin,
	str::FromStr,
	task::{Context, Poll},
	time::{SystemTime, UNIX_EPOCH},
	usize,
};

use argan_core::{
	body::{Body, Frame, HttpBody},
	BoxedError,
};
use brotli::CompressorReader;
use bytes::{BufMut, Bytes, BytesMut};
use flate2::{
	read::{DeflateEncoder, GzEncoder},
	Compression,
};
use http::{
	header::{
		ACCEPT_RANGES, CONTENT_DISPOSITION, CONTENT_ENCODING, CONTENT_LENGTH, CONTENT_RANGE,
		CONTENT_TYPE,
	},
	HeaderValue, StatusCode,
};
use percent_encoding::{percent_encode, NON_ALPHANUMERIC};
use rand::{rngs::SmallRng, Rng, SeedableRng};
use tokio::task::JoinError;

use crate::{
	common::{IntoArray, SCOPE_VALIDITY},
	response::{IntoResponse, Response},
};

// --------------------------------------------------

pub mod config;

use config::{ConfigFlags, FileStreamConfigOption};
pub use config::{
	_as_attachment, _boundary, _content_encoding, _content_type, _file_name,
	_to_support_partial_content,
};

#[doc(inline)]
pub use config::ContentCoding;

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

const BUFFER_SIZE: usize = 8 * 1024;
const BROTLI_LG_WINDOW_SIZE: usize = 22; // Recommended size (brotli crate).

// -------------------------

/// A low-level primitive to stream files with support for `multipart/byteranges` and
/// dynamic encoding.
///
/// Streaming should be used on files that won't be locked. An instance of a `FileStream`
/// can be used as a response or as a response body.
pub struct FileStream {
	maybe_encoded_file: MaybeEncoded,
	file_size: Box<str>,
	current_range_index: usize,
	current_range_size: u64,
	current_range_remaining_size: u64,
	ranges: Vec<RangeValue>,
	some_content_encoding: Option<HeaderValue>,
	some_content_type: Option<HeaderValue>,
	some_boundary: Option<Box<str>>,
	some_file_name: Option<Box<str>>,
	config_flags: ConfigFlags,
}

impl FileStream {
	/// Opens the file at the given path for streaming.
	pub async fn open<P>(path: P) -> Result<Self, FileStreamError>
	where
		P: AsRef<Path> + Send + 'static,
	{
		let result = tokio::task::spawn_blocking(|| {
			let file = File::open(path)?;

			let metadata = file.metadata()?;
			let file_size = metadata.len();

			Result::<_, FileStreamError>::Ok((file, file_size))
		})
		.await?;

		let (file, file_size) = result?;

		Ok(Self {
			maybe_encoded_file: MaybeEncoded::Identity(file),
			file_size: file_size.to_string().into(),
			current_range_index: 0,
			current_range_size: file_size,
			current_range_remaining_size: file_size,
			ranges: Vec::new(),
			some_content_encoding: None,
			some_content_type: None,
			some_boundary: None,
			some_file_name: None,
			config_flags: ConfigFlags::NONE,
		})
	}

	// Opens the file at the given path for streaming with dynamic encoding.
	pub async fn open_with_encoding<P>(
		path: P,
		content_coding: ContentCoding,
	) -> Result<Self, FileStreamError>
	where
		P: AsRef<Path> + Send + 'static,
	{
		let result = tokio::task::spawn_blocking(|| {
			let file = File::open(path)?;

			let metadata = file.metadata()?;
			let file_size = metadata.len();

			Result::<_, FileStreamError>::Ok((file, file_size))
		})
		.await?;

		let (file, file_size) = result?;

		let (maybe_encoded_file, content_encoding) = match content_coding {
			ContentCoding::Gzip(level) => (
				MaybeEncoded::Gzip(GzEncoder::new(file, Compression::new(level))),
				HeaderValue::from_static("gzip"),
			),
			ContentCoding::Deflate(level) => (
				MaybeEncoded::Deflate(DeflateEncoder::new(file, Compression::new(level))),
				HeaderValue::from_static("deflate"),
			),
			ContentCoding::Brotli(level) => (
				MaybeEncoded::Brotli(CompressorReader::new(
					file,
					BUFFER_SIZE,
					level,
					BROTLI_LG_WINDOW_SIZE as u32,
				)),
				HeaderValue::from_static("br"),
			),
		};

		Ok(Self {
			maybe_encoded_file,
			file_size: file_size.to_string().into(),
			current_range_index: 0,
			current_range_size: file_size,
			current_range_remaining_size: file_size,
			ranges: Vec::new(),
			some_content_encoding: Some(content_encoding),
			some_content_type: None,
			some_boundary: None,
			some_file_name: None,
			config_flags: ConfigFlags::NONE,
		})
	}

	/// Creates a stream from a given file.
	pub async fn from_file(file: File) -> Result<Self, FileStreamError> {
		let result = tokio::task::spawn_blocking(|| {
			let metadata = file.metadata()?;
			let file_size = metadata.len();

			Result::<_, FileStreamError>::Ok((file, file_size))
		})
		.await?;

		let (file, file_size) = result?;

		Ok(Self {
			maybe_encoded_file: MaybeEncoded::Identity(file),
			file_size: file_size.to_string().into(),
			current_range_index: 0,
			current_range_size: file_size,
			current_range_remaining_size: file_size,
			ranges: Vec::new(),
			some_content_encoding: None,
			some_content_type: None,
			some_boundary: None,
			some_file_name: None,
			config_flags: ConfigFlags::NONE,
		})
	}

	/// Creates a stream from a given file with dynamic encoding.
	pub async fn from_file_with_encoding(
		file: File,
		content_coding: ContentCoding,
	) -> Result<Self, FileStreamError> {
		let result = tokio::task::spawn_blocking(|| {
			let metadata = file.metadata()?;
			let file_size = metadata.len();

			Result::<_, FileStreamError>::Ok((file, file_size))
		})
		.await?;

		let (file, file_size) = result?;

		let (maybe_encoded_file, content_encoding) = match content_coding {
			ContentCoding::Gzip(level) => (
				MaybeEncoded::Gzip(GzEncoder::new(file, Compression::new(level))),
				HeaderValue::from_static("gzip"),
			),
			ContentCoding::Deflate(level) => (
				MaybeEncoded::Deflate(DeflateEncoder::new(file, Compression::new(level))),
				HeaderValue::from_static("deflate"),
			),
			ContentCoding::Brotli(level) => (
				MaybeEncoded::Brotli(CompressorReader::new(
					file,
					BUFFER_SIZE,
					level,
					BROTLI_LG_WINDOW_SIZE as u32,
				)),
				HeaderValue::from_static("br"),
			),
		};

		Ok(Self {
			maybe_encoded_file,
			file_size: file_size.to_string().into(),
			current_range_index: 0,
			current_range_size: file_size,
			current_range_remaining_size: file_size,
			ranges: Vec::new(),
			some_content_encoding: Some(content_encoding),
			some_content_type: None,
			some_boundary: None,
			some_file_name: None,
			config_flags: ConfigFlags::NONE,
		})
	}

	/// Opens the file at the given path for streaming some of its parts.
	pub async fn open_ranges<P>(
		path: P,
		range_header_value: &str,
		allow_descending: bool,
	) -> Result<Self, FileStreamError>
	where
		P: AsRef<Path> + Send + 'static,
	{
		if range_header_value.is_empty() {
			return Err(FileStreamError::InvalidRangeValue);
		}

		let result = tokio::task::spawn_blocking(|| {
			let file = File::open(path)?;

			let metadata = file.metadata()?;
			let file_size = metadata.len();

			Result::<_, FileStreamError>::Ok((file, file_size))
		})
		.await?;

		let (file, file_size) = result?;

		let ranges = parse_range_header_value(range_header_value, file_size, allow_descending)?;

		Ok(Self {
			maybe_encoded_file: MaybeEncoded::Identity(file),
			file_size: file_size.to_string().into(),
			current_range_index: 0,
			current_range_size: ranges[0].size(),
			current_range_remaining_size: ranges[0].size(),
			ranges,
			some_content_encoding: None,
			some_content_type: None,
			some_boundary: None,
			some_file_name: None,
			config_flags: ConfigFlags::PARTIAL_CONTENT_SUPPORT,
		})
	}

	/// Creates a stream from some parts of the given file.
	pub async fn from_file_ranges(
		file: File,
		range_header_value: &str,
		allow_descending: bool,
	) -> Result<Self, FileStreamError> {
		if range_header_value.is_empty() {
			return Err(FileStreamError::InvalidRangeValue);
		}

		let result = tokio::task::spawn_blocking(|| {
			let metadata = file.metadata()?;
			let file_size = metadata.len();

			Result::<_, FileStreamError>::Ok((file, file_size))
		})
		.await?;

		let (file, file_size) = result?;

		let ranges = parse_range_header_value(range_header_value, file_size, allow_descending)?;

		Ok(Self {
			maybe_encoded_file: MaybeEncoded::Identity(file),
			file_size: file_size.to_string().into(),
			current_range_index: 0,
			current_range_size: ranges[0].size(),
			current_range_remaining_size: ranges[0].size(),
			ranges,
			some_content_encoding: None,
			some_content_type: None,
			some_boundary: None,
			some_file_name: None,
			config_flags: ConfigFlags::PARTIAL_CONTENT_SUPPORT,
		})
	}

	/// Configures the FileStream with the given options.
	pub fn configure<C, const N: usize>(&mut self, config_options: C)
	where
		C: IntoArray<FileStreamConfigOption, N>,
	{
		let config_options = config_options.into_array();

		for config_option in config_options {
			use FileStreamConfigOption::*;

			match config_option {
				Attachment => self.config_flags.add(ConfigFlags::ATTACHMENT),
				PartialContentSupport => {
					if self.maybe_encoded_file.is_encoded() {
						panic!("cannot support partial content with dynamic encoding");
					}

					self.config_flags.add(ConfigFlags::PARTIAL_CONTENT_SUPPORT);
				}
				ContentEncoding(header_value) => {
					let encoding = header_value
						.to_str()
						.expect("Content-Encoding must be a valid header value");
					match self.maybe_encoded_file {
						MaybeEncoded::Uninitialized => unreachable!(),
						MaybeEncoded::Identity(_) => {}
						MaybeEncoded::Gzip(_) => {
							if encoding != "gzip" {
								panic!(
									"applied dynamic encoding `gzip` and Content-Encoding `{}` are different",
									encoding
								);
							}
						}
						MaybeEncoded::Deflate(_) => {
							if encoding != "deflate" {
								panic!(
									"applied dynamic encoding `deflate` and Content-Encoding `{}` are different",
									encoding
								);
							}
						}
						MaybeEncoded::Brotli(_) => {
							if encoding != "br" {
								panic!(
									"applied dynamic encoding `brotli` and Content-Encoding `{}` are different",
									encoding
								);
							}
						}
					}

					self.some_content_encoding = Some(header_value);
				}
				ContentType(header_value) => self.some_content_type = Some(header_value),
				Boundary(boundary) => {
					if self.maybe_encoded_file.is_encoded() {
						panic!("boundary cannot be set with dynamic encoding");
					}

					if boundary.len() > 70 {
						panic!("boundary exceeds 70 characters");
					}

					if boundary.chars().any(|ch| !ch.is_ascii_graphic()) {
						panic!("boundary contains non-graphic character");
					}

					self.some_boundary = Some(boundary);
					self.config_flags.add(ConfigFlags::PARTIAL_CONTENT_SUPPORT);
				}
				FileName(file_name) => {
					let mut file_name_string = String::new();
					file_name_string.push_str("; filename");

					if file_name
						.as_bytes()
						.iter()
						.any(|ch| !ch.is_ascii_alphanumeric())
					{
						file_name_string.push_str("*=utf-8''");
						file_name_string
							.push_str(&percent_encode(file_name.as_bytes(), NON_ALPHANUMERIC).to_string());
					} else {
						file_name_string.push_str("=\"");
						file_name_string.push_str(file_name.as_ref());
						file_name_string.push('"');
					}

					self.some_file_name = Some(file_name_string.into());
				}
			}
		}
	}
}

impl IntoResponse for FileStream {
	fn into_response(mut self) -> Response {
		if self.some_boundary.is_some() {
			multipart_ranges_response(self)
		} else if self.ranges.len() > 1 {
			self.some_boundary = Some(
				generate_boundary(48)
					.expect("boundary should be valid when the length is no longer than 70 characters"),
			);

			multipart_ranges_response(self)
		} else {
			single_range_response(self)
		}
	}
}

macro_rules! insert_header {
	($response:ident, $header_name:ident, $header_value:expr) => {
		if let Ok(header_value) = HeaderValue::from_str($header_value) {
			$response.headers_mut().insert($header_name, header_value);
		} else {
			return StatusCode::INTERNAL_SERVER_ERROR.into_response();
		}
	};
}

fn single_range_response(mut file_stream: FileStream) -> Response {
	let mut response = Response::default();
	if let Some(content_type_value) = file_stream.some_content_type.take() {
		response
			.headers_mut()
			.insert(CONTENT_TYPE, content_type_value);
	}

	if let Some(content_encoding_value) = file_stream.some_content_encoding.take() {
		response
			.headers_mut()
			.insert(CONTENT_ENCODING, content_encoding_value);
	}

	// Non-multipart responses may have a single range. It's expected to be checked before
	// the function is called.
	if !file_stream.ranges.is_empty() {
		*response.status_mut() = StatusCode::PARTIAL_CONTENT;

		let mut content_range_value = String::new();
		content_range_value.push_str("bytes ");
		content_range_value.push_str(file_stream.ranges[0].start_str());
		content_range_value.push('-');
		content_range_value.push_str(file_stream.ranges[0].end_str());
		content_range_value.push('/');
		content_range_value.push_str(&file_stream.file_size);

		insert_header!(response, CONTENT_RANGE, &content_range_value);
	} else {
		let some_content_disposition_value = if file_stream.config_flags.has(ConfigFlags::ATTACHMENT) {
			let mut content_disposition_value = String::new();
			content_disposition_value.push_str("attachment");

			if let Some(file_name) = file_stream.some_file_name.take() {
				content_disposition_value.push_str(&file_name);
			}

			Some(content_disposition_value)
		} else if let Some(file_name) = file_stream.some_file_name.take() {
			let mut content_disposition_value = String::new();
			content_disposition_value.push_str("inline");
			content_disposition_value.push_str(&file_name);

			Some(content_disposition_value)
		} else {
			None
		};

		if let Some(content_disposition_value) = some_content_disposition_value {
			insert_header!(response, CONTENT_DISPOSITION, &content_disposition_value);
		}

		if let MaybeEncoded::Identity(_) = file_stream.maybe_encoded_file {
			if file_stream
				.config_flags
				.has(ConfigFlags::PARTIAL_CONTENT_SUPPORT)
			{
				response
					.headers_mut()
					.insert(ACCEPT_RANGES, HeaderValue::from_static("bytes"));
			}
		}
	}

	if let MaybeEncoded::Identity(_) = file_stream.maybe_encoded_file {
		// current_range_remaining_size is either a file size or a single range size.
		insert_header!(
			response,
			CONTENT_LENGTH,
			&file_stream.current_range_remaining_size.to_string()
		);
	} else {
		response
			.headers_mut()
			.insert(ACCEPT_RANGES, HeaderValue::from_static("none"));
	}

	*response.body_mut() = Body::new(file_stream);

	response
}

fn multipart_ranges_response(mut file_stream: FileStream) -> Response {
	let boundary = file_stream
		.some_boundary
		.as_ref()
		.expect("streaming multipart ranges shouldn't start without a boundary")
		.clone();

	let mut response = Response::default();
	*response.status_mut() = StatusCode::PARTIAL_CONTENT;

	{
		let value_part = "multipart/byteranges; boundary=";
		let mut header_value = String::with_capacity(value_part.len() + boundary.len());
		header_value.push_str(value_part);
		header_value.push_str(&boundary);

		insert_header!(response, CONTENT_TYPE, &header_value);
	}

	if let Some(content_encoding) = file_stream.some_content_encoding.take() {
		response
			.headers_mut()
			.insert(CONTENT_ENCODING, content_encoding);
	}

	let mut body_size = 0u64;
	if !file_stream.ranges.is_empty() {
		for range in file_stream.ranges.iter() {
			body_size += part_header_size(
				boundary.len(),
				file_stream.some_content_type.as_ref(),
				Some(range),
				&file_stream.file_size,
			) as u64;

			body_size += range.size();
		}
	} else {
		body_size += part_header_size(
			boundary.len(),
			file_stream.some_content_type.as_ref(),
			None,
			&file_stream.file_size,
		) as u64;

		// Where there is no range, the current_range_remaining_size is set to the file size.
		body_size += file_stream.current_range_remaining_size;
	}

	//           |     |       |     |
	//           \r\n--boundary--\r\n
	body_size += (4 + boundary.len() + 4) as u64;

	insert_header!(response, CONTENT_LENGTH, &body_size.to_string());

	*response.body_mut() = Body::new(file_stream);

	response
}

#[inline]
fn part_header_size(
	boundary_length: usize,
	some_content_type_value: Option<&HeaderValue>,
	some_range: Option<&RangeValue>,
	file_size: &str,
) -> usize {
	// |     |       |
	// \r\n--boundary
	let mut part_header_size = 4 + boundary_length;

	if let Some(content_type_value) = some_content_type_value {
		// |   |           | |    |
		// \r\nContent-Type: value
		part_header_size += 2 + CONTENT_TYPE.as_str().len() + 2 + content_type_value.len();
	}

	// |   |            |       |
	// \r\nContent-Range: bytes
	part_header_size += 2 + CONTENT_RANGE.as_str().len() + 8;

	if let Some(range) = some_range {
		// |        ||            |       |
		// range len/file size len\r\n\r\n
		part_header_size += range.len() + 1 + file_size.len() + 4;
	} else {
		// | |            ||            |       |
		// 0-file_size len/file size len\r\n\r\n
		part_header_size += 2 + file_size.len() + 1 + file_size.len() + 4;
	}

	part_header_size
}

// ----------

impl HttpBody for FileStream {
	type Data = Bytes;
	type Error = BoxedError;

	fn poll_frame(
		mut self: Pin<&mut Self>,
		cx: &mut Context<'_>,
	) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
		if self.some_boundary.is_some() {
			stream_multipart_ranges(self, cx)
		} else if self.ranges.len() > 1 {
			self.some_boundary = Some(
				generate_boundary(48)
					.expect("boundary should be valid when the length is no longer than 70 characters"),
			);

			stream_multipart_ranges(self, cx)
		} else {
			stream_single_range(self, cx)
		}
	}
}

fn stream_single_range(
	mut file_stream: Pin<&mut FileStream>,
	cx: &mut Context<'_>,
) -> Poll<Option<Result<Frame<Bytes>, BoxedError>>> {
	for _ in 0..3 {
		let mut buffer = if let MaybeEncoded::Identity(_) = &mut file_stream.maybe_encoded_file {
			if file_stream.current_range_remaining_size == 0 {
				return Poll::Ready(None);
			}

			// Use `current_range_index` to seek only once.
			if file_stream.current_range_index == 0 && file_stream.ranges.len() == 1 {
				let start_position = file_stream.ranges[0].start();
				let _ = file_stream
					.maybe_encoded_file
					.seek(std::io::SeekFrom::Start(start_position))
					.expect("start position must be less than the file size");

				file_stream.current_range_index += 1;
			}

			if file_stream.current_range_remaining_size < BUFFER_SIZE as u64 {
				BytesMut::zeroed(file_stream.current_range_remaining_size as usize)
			} else {
				BytesMut::zeroed(BUFFER_SIZE)
			}
		} else {
			BytesMut::zeroed(BUFFER_SIZE)
		};

		match file_stream.maybe_encoded_file.read(&mut buffer) {
			Ok(size) => {
				buffer.truncate(size);

				if let MaybeEncoded::Identity(_) = file_stream.maybe_encoded_file {
					file_stream.current_range_remaining_size -= size as u64;
				} else if size == 0 {
					// We can't know the remaining size with dynamic encoding.
					return Poll::Ready(None);
				}

				return Poll::Ready(Some(Ok(Frame::data(buffer.freeze()))));
			}
			Err(error) => {
				if error.kind() != ErrorKind::Interrupted {
					return Poll::Ready(Some(Err(error.into())));
				}
			}
		}
	}

	cx.waker().wake_by_ref();

	Poll::Pending
}

fn stream_multipart_ranges(
	mut file_stream: Pin<&mut FileStream>,
	cx: &mut Context<'_>,
) -> Poll<Option<Result<Frame<Bytes>, BoxedError>>> {
	let new_part = if file_stream.current_range_remaining_size == 0 {
		let next_range_index = file_stream.current_range_index + 1;
		if let Some(next_range_size) = file_stream
			.ranges
			.get(next_range_index)
			.map(|next_range| next_range.size())
		{
			file_stream.current_range_index = next_range_index;
			file_stream.current_range_size = next_range_size;
			file_stream.current_range_remaining_size = next_range_size;

			true
		} else {
			return Poll::Ready(None);
		}
	} else {
		file_stream.current_range_index == 0
			&& file_stream.current_range_remaining_size == file_stream.current_range_size
	};

	let boundary = file_stream
		.some_boundary
		.as_ref()
		.expect("streaming multipart ranges shouldn't start without a boundary")
		.clone();

	let (capacity, ending_length) = if file_stream.current_range_remaining_size <= BUFFER_SIZE as u64
	{
		let capacity = file_stream.current_range_remaining_size as usize;

		if file_stream.ranges.is_empty()
			|| file_stream.current_range_index == file_stream.ranges.len() - 1
		{
			//                  |     |       |     |
			//                  \r\n--boundary--\r\n
			let ending_length = 4 + boundary.len() + 4;
			(capacity + ending_length, ending_length)
		} else {
			(capacity, 0)
		}
	} else {
		(BUFFER_SIZE, 0)
	};

	let (mut buffer, start_index) = if new_part {
		let some_range = if !file_stream.ranges.is_empty() {
			let current_range_start = file_stream.ranges[file_stream.current_range_index].start();
			let _ = file_stream
				.maybe_encoded_file
				.seek(SeekFrom::Start(current_range_start))
				.expect("range start must be less than the file size");

			Some(&file_stream.ranges[file_stream.current_range_index])
		} else {
			None
		};

		let part_header_size = part_header_size(
			boundary.len(),
			file_stream.some_content_type.as_ref(),
			some_range,
			&file_stream.file_size,
		);

		let mut buffer = BytesMut::with_capacity(capacity + part_header_size);
		buffer.put_slice(b"\r\n--");
		buffer.put_slice(boundary.as_bytes());

		if let Some(content_type_value) = file_stream.some_content_type.as_ref() {
			buffer.put_slice(b"\r\n");
			buffer.put_slice(CONTENT_TYPE.as_str().as_bytes());
			buffer.put_slice(b": ");
			buffer.put_slice(content_type_value.as_bytes());
		}

		buffer.put_slice(b"\r\n");
		buffer.put_slice(CONTENT_RANGE.as_str().as_bytes());
		buffer.put_slice(b": bytes ");
		buffer.put_slice(some_range.map_or(b"0", |range| range.start_str().as_bytes()));
		buffer.put_u8(b'-');
		buffer.put_slice(
			some_range.map_or(file_stream.file_size.as_bytes(), |range| {
				range.end_str().as_bytes()
			}),
		);
		buffer.put_u8(b'/');
		buffer.put_slice(file_stream.file_size.as_bytes());
		buffer.put_slice(b"\r\n\r\n");

		let filled_length = buffer.len();
		buffer.resize(capacity + part_header_size, 0);

		(buffer, filled_length)
	} else {
		(BytesMut::zeroed(capacity), 0)
	};

	let end_index = buffer.len() - ending_length;

	for _ in 0..3 {
		match file_stream
			.maybe_encoded_file
			.read(&mut buffer[start_index..end_index])
		{
			Ok(size) => {
				file_stream.current_range_remaining_size -= size as u64;
				buffer.resize(start_index + size, 0);

				if ending_length > 0 && file_stream.current_range_remaining_size == 0 {
					buffer.put_slice(b"\r\n--");
					buffer.put_slice(boundary.as_bytes());
					buffer.put_slice(b"--\r\n");
				}

				return Poll::Ready(Some(Ok(Frame::data(buffer.freeze()))));
			}
			Err(error) => {
				if error.kind() != ErrorKind::Interrupted {
					return Poll::Ready(Some(Err(error.into())));
				}
			}
		}
	}

	cx.waker().wake_by_ref();

	Poll::Pending
}

/// Generates a boundary with the given length.
///
/// # Panics
/// - if the length is greater than 70
pub fn generate_boundary(length: u8) -> Result<Box<str>, FileStreamError> {
	if length == 0 || length > 70 {
		panic!("length is not in the range of 1..=70");
	}

	let now = SystemTime::now();
	let seed = if let Ok(duration) = now.duration_since(UNIX_EPOCH) {
		duration.as_secs()
	} else {
		UNIX_EPOCH
			.duration_since(now)
			.expect("This is fine!")
			.as_secs()
	};

	let mut small_rng = SmallRng::seed_from_u64(seed);
	let mut boundary_string = String::with_capacity(length as usize);
	for _ in 0..length {
		let ch = char::from_u32(small_rng.gen_range(32..=126u32))
			.expect("printable ASCII range should be a valid char");

		boundary_string.push(ch)
	}

	Ok(boundary_string.into())
}

// ----------

enum MaybeEncoded {
	Uninitialized,
	Identity(File),
	Gzip(GzEncoder<File>),
	Deflate(DeflateEncoder<File>),
	Brotli(CompressorReader<File>),
}

impl MaybeEncoded {
	fn apply_coding(&mut self, content_coding: ContentCoding) -> Result<(), FileStreamError> {
		let owned_self = std::mem::replace(self, Self::Uninitialized);
		match owned_self {
			Self::Uninitialized => unreachable!(),
			Self::Identity(file) => match content_coding {
				ContentCoding::Gzip(level) => {
					*self = Self::Gzip(GzEncoder::new(file, Compression::new(level)))
				}
				ContentCoding::Deflate(level) => {
					*self = Self::Deflate(DeflateEncoder::new(file, Compression::new(level)))
				}
				ContentCoding::Brotli(level) => {
					*self = Self::Brotli(CompressorReader::new(
						file,
						BUFFER_SIZE,
						level,
						BROTLI_LG_WINDOW_SIZE as u32,
					))
				}
			},
			_ => panic!("content coding has already been applied"),
		}

		Ok(())
	}

	#[inline(always)]
	fn is_encoded(&self) -> bool {
		match self {
			Self::Uninitialized => unreachable!(),
			Self::Identity(_) => false,
			_ => true,
		}
	}
}

impl Read for MaybeEncoded {
	fn read(&mut self, buf: &mut [u8]) -> Result<usize, IoError> {
		match self {
			Self::Uninitialized => unreachable!(),
			Self::Identity(file) => file.read(buf),
			Self::Brotli(br_encoder) => br_encoder.read(buf),
			Self::Gzip(gzip_encoder) => gzip_encoder.read(buf),
			Self::Deflate(deflate_encoder) => deflate_encoder.read(buf),
		}
	}
}

impl Seek for MaybeEncoded {
	fn seek(&mut self, pos: SeekFrom) -> Result<u64, IoError> {
		match self {
			Self::Uninitialized => unreachable!(),
			Self::Identity(file) => file.seek(pos),
			_ => panic!("cannot call seek on a file with content encoding"),
		}
	}
}

// ----------

fn parse_range_header_value(
	value: &str,
	file_size: u64,
	allow_descending: bool,
) -> Result<Vec<RangeValue>, FileStreamError> {
	let Some(ranges_str) = value.strip_prefix("bytes=") else {
		return Err(FileStreamError::InvalidRangeValue);
	};

	let mut raw_ranges = ranges_str
		.split(',')
		.filter_map(|range_str| {
			RawRangeValue::from_str(range_str.trim())
				.and_then(|range| {
					if range
						.some_start
						.as_ref()
						.is_some_and(|start| start.0 < file_size)
					{
						return Ok(range);
					}

					if range.some_start.is_none() && range.some_end.as_ref().is_some_and(|end| end.0 > 0) {
						return Ok(range);
					}

					Err(FileStreamError::UnsatisfiableRange)
				})
				.ok()
		})
		.collect::<Vec<RawRangeValue>>();

	if raw_ranges.is_empty() {
		return Err(FileStreamError::UnsatisfiableRange);
	}

	if raw_ranges.len() == 1 {
		let file_end = file_size - 1;

		let mut raw_range = raw_ranges.pop().expect(SCOPE_VALIDITY);
		let range = if raw_range.some_start.is_none() {
			let end = raw_range
				.some_end
				.take()
				.expect("suffix range must have a valid end length");

			if end.0 < file_size {
				RangeValue::new(file_size - end.0, file_end)
			} else {
				RangeValue::new(0, file_end)
			}
		} else if raw_range
			.some_end
			.as_ref()
			.is_some_and(|end| end.0 < file_size)
		{
			RangeValue::try_from(raw_range).expect(SCOPE_VALIDITY)
		} else {
			RangeValue {
				start: raw_range.some_start.expect(SCOPE_VALIDITY),
				end: (file_end, file_end.to_string().into()),
			}
		};

		return Ok(vec![range]);
	}

	let ascending_range = if let Some(first_position) = raw_ranges
		.iter()
		.position(|range| range.some_start.is_some())
	{
		let first_start = raw_ranges[first_position]
			.some_start
			.as_ref()
			.expect(SCOPE_VALIDITY);

		let start_index = first_position + 1;
		if let Some(second_position) = raw_ranges[start_index..]
			.iter()
			.position(|range| range.some_start.is_some())
			.map(|position| position + start_index)
		{
			let second_start = raw_ranges[second_position]
				.some_start
				.as_ref()
				.expect(SCOPE_VALIDITY);

			first_start.0 < second_start.0
		} else {
			true // We have a single range with a start value that we're considering as ascending.
		}
	} else {
		false // We have only suffix length values.
	};

	if !ascending_range && !allow_descending {
		return Err(FileStreamError::UnsatisfiableRange);
	}

	let (valid_ranges, some_biggest_suffix_range) =
		get_valid_rangges(raw_ranges, ascending_range, file_size)?;

	Ok(combine_valid_and_suffix_ranges(
		valid_ranges,
		some_biggest_suffix_range,
		ascending_range,
		file_size,
	))
}

#[inline(always)]
fn get_valid_rangges(
	ranges: Vec<RawRangeValue>,
	ascending_range: bool,
	file_size: u64,
) -> Result<(Vec<RangeValue>, Option<RawRangeValue>), FileStreamError> {
	let mut valid_ranges = Vec::<RangeValue>::new();
	let mut overlap_count = 0;
	let mut range_with_file_end_exists = false;
	let mut some_biggest_suffix_range = None;

	for mut range in ranges {
		if let Some(current_start) = range.some_start.take() {
			if let Some(previous_start) = valid_ranges.last().map(|range| range.start.0) {
				// If ranges were descending and this wasn't allowed
				// we would have returned an error so far.
				// Here we're checking if they are mixed or not.
				if current_start.0 < previous_start {
					if ascending_range {
						return Err(FileStreamError::UnsatisfiableRange);
					}
				} else if !ascending_range {
					return Err(FileStreamError::UnsatisfiableRange);
				}
			}

			if range_with_file_end_exists && ascending_range {
				// There is no point in going through the remaining non-suffix ranges because,
				// from this range, we have to return the remaining chunk of the file as a whole,
				// unless some suffix range requires starting from an earlier position.
				continue;
			}

			let current_end = if range
				.some_end
				.as_ref()
				.is_some_and(|current_end| current_end.0 < file_size)
			{
				range.some_end.expect(SCOPE_VALIDITY)
			} else {
				range_with_file_end_exists = true;
				let file_end = file_size - 1;

				(file_end, file_end.to_string().into())
			};

			let mut merge = false;

			// Merge conditions.
			if ascending_range {
				if let Some(previous_end) = valid_ranges.last().map(|range| range.end.0) {
					if current_start.0 < previous_end {
						if overlap_count > 1 {
							return Err(FileStreamError::UnsatisfiableRange);
						}

						overlap_count += 1;
						merge = true;
					} else if current_start.0 - previous_end < 128 {
						merge = true;
					}
				}
			} else {
				if let Some(position) = valid_ranges
					.iter()
					.position(|range| range.start.0 <= current_end.0)
				{
					valid_ranges.truncate(position + 1);
				}

				if let Some(previous_start) = valid_ranges.last().map(|range| range.start.0) {
					if previous_start < current_end.0 {
						if overlap_count > 1 {
							return Err(FileStreamError::UnsatisfiableRange);
						}

						overlap_count += 1;
						merge = true;
					} else if previous_start - current_end.0 < 128 {
						merge = true;
					}
				}
			}

			if merge {
				if let Some(range) = valid_ranges.last_mut() {
					if !ascending_range {
						range.start = current_start;
					}

					if range.end.0 < current_end.0 {
						range.end = current_end;
					}
				}
			} else {
				valid_ranges.push(RangeValue {
					start: current_start,
					end: current_end,
				});
			}
		} else {
			// Finding the biggest suffix range.
			let current_end = range
				.some_end
				.expect("suffix range must have a valid end length");

			if some_biggest_suffix_range
				.as_ref()
				.map_or(true, |range: &RawRangeValue| {
					range.some_end.as_ref().expect(SCOPE_VALIDITY).0 < current_end.0
				}) {
				if current_end.0 < file_size {
					some_biggest_suffix_range = Some(RawRangeValue {
						some_start: None,
						some_end: Some(current_end),
					});
				} else {
					return Ok((vec![RangeValue::new(0, file_size - 1)], None));
				}
			}
		}
	}

	Ok((valid_ranges, some_biggest_suffix_range))
}

#[inline(always)]
fn combine_valid_and_suffix_ranges(
	mut valid_ranges: Vec<RangeValue>,
	some_biggest_suffix_range: Option<RawRangeValue>,
	ascending_range: bool,
	file_size: u64,
) -> Vec<RangeValue> {
	if let Some(suffix_range) = some_biggest_suffix_range {
		let end_length = suffix_range
			.some_end
			.as_ref()
			.expect("suffix range must have a valid end length")
			.0;

		let suffix_range_start = file_size - end_length;
		let check_position = |(i, range): (usize, &RangeValue)| {
			if range.end.0 < suffix_range_start {
				Some((i, true, range.end.0))
			} else if range.start.0 < suffix_range_start {
				Some((i, false, 0))
			} else {
				None
			}
		};

		let file_end = file_size - 1;

		if ascending_range {
			if let Some((position, bigger_than_end, end)) = valid_ranges
				.iter()
				.enumerate()
				.rev()
				.find_map(check_position)
			{
				valid_ranges.truncate(position + 1);

				if !bigger_than_end || suffix_range_start - end < 128 {
					valid_ranges[position].end = (file_end, file_end.to_string().into());
				} else {
					valid_ranges.push(RangeValue::new(suffix_range_start, file_end))
				}
			} else {
				valid_ranges.clear();
				valid_ranges.push(RangeValue::new(0, file_end));
			}
		} else if let Some((mut position, bigger_than_end, end)) =
			valid_ranges.iter().enumerate().find_map(check_position)
		{
			if !bigger_than_end || suffix_range_start - end < 128 {
				valid_ranges[position].end = (file_end, file_end.to_string().into());
				valid_ranges.rotate_left(position);
				valid_ranges.truncate(valid_ranges.len() - position);
			} else if position > 0 {
				position -= 1;
				valid_ranges[position] = RangeValue::new(suffix_range_start, file_end);
				valid_ranges.rotate_left(position);
				valid_ranges.truncate(valid_ranges.len() - position);
			} else {
				valid_ranges.insert(position, RangeValue::new(suffix_range_start, file_end));
			}
		} else {
			valid_ranges.clear();
			valid_ranges.push(RangeValue::new(suffix_range_start, file_end));
		}
	}

	valid_ranges
}

// --------------------------------------------------

#[derive(Debug, Eq)]
struct RawRangeValue {
	some_start: Option<(u64, Box<str>)>,
	some_end: Option<(u64, Box<str>)>,
}

impl FromStr for RawRangeValue {
	type Err = FileStreamError;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		let Some((start, end)) = s.split_once('-') else {
			return Err(FileStreamError::InvalidRangeValue);
		};

		let some_start_u64 = if !start.is_empty() {
			if let Ok(start_u64) = start.parse::<u64>() {
				Some(start_u64)
			} else {
				return Err(FileStreamError::InvalidRangeValue);
			}
		} else {
			None
		};

		let some_end_u64 = if !end.is_empty() {
			if let Ok(end_64) = end.parse::<u64>() {
				Some(end_64)
			} else {
				return Err(FileStreamError::InvalidRangeValue);
			}
		} else {
			None
		};

		if some_end_u64
			.is_some_and(|end_u64| some_start_u64.is_some_and(|start_u64| end_u64 < start_u64))
		{
			return Err(FileStreamError::UnsatisfiableRange);
		}

		Ok(RawRangeValue {
			some_start: some_start_u64.map(|start_u64| (start_u64, start.into())),
			some_end: some_end_u64.map(|end_u64| (end_u64, end.into())),
		})
	}
}

impl PartialEq for RawRangeValue {
	fn eq(&self, other: &Self) -> bool {
		self.some_start.as_ref().is_some_and(|self_start| {
			other
				.some_start
				.as_ref()
				.is_some_and(|other_start| other_start.0 == self_start.0)
		}) && self.some_end.as_ref().is_some_and(|self_end| {
			other
				.some_end
				.as_ref()
				.is_some_and(|other_end| other_end.0 == self_end.0)
		})
	}
}

// ----------

#[derive(Debug, Eq)]
struct RangeValue {
	start: (u64, Box<str>),
	end: (u64, Box<str>),
}

impl RangeValue {
	#[inline(always)]
	pub fn new(start: u64, end: u64) -> Self {
		Self {
			start: (start, start.to_string().into()),
			end: (end, end.to_string().into()),
		}
	}

	#[inline(always)]
	pub fn start(&self) -> u64 {
		self.start.0
	}

	#[inline(always)]
	pub fn start_str(&self) -> &str {
		self.start.1.as_ref()
	}

	#[inline(always)]
	pub fn end(&self) -> u64 {
		self.end.0
	}

	#[inline(always)]
	pub fn end_str(&self) -> &str {
		self.end.1.as_ref()
	}

	#[inline(always)]
	pub fn len(&self) -> usize {
		// |    ||  |
		// start-end
		self.start.1.len() + 1 + self.end.1.len()
	}

	#[inline(always)]
	pub fn size(&self) -> u64 {
		// Invariant: start <= end.
		self.end.0 - self.start.0 + 1 // End is inclusive.
	}
}

impl TryFrom<RawRangeValue> for RangeValue {
	type Error = FileStreamError;

	fn try_from(value: RawRangeValue) -> Result<Self, Self::Error> {
		if value.some_start.is_none() || value.some_end.is_none() {
			return Err(FileStreamError::InvalidRangeValue);
		}

		let start = value.some_start.expect(SCOPE_VALIDITY);
		let end = value.some_end.expect(SCOPE_VALIDITY);

		Ok(Self { start, end })
	}
}

impl PartialEq for RangeValue {
	fn eq(&self, other: &Self) -> bool {
		self.start.0 == other.start.0 && self.end.0 == other.end.0
	}
}

// --------------------------------------------------
// FileStreamError

/// An error that's returned when creating a file stream fails.
#[non_exhaustive]
#[derive(Debug, crate::ImplError)]
pub enum FileStreamError {
	#[error(transparent)]
	Io(#[from] IoError),
	#[error(transparent)]
	Runtime(#[from] JoinError),
	#[error("invalid range value")]
	InvalidRangeValue,
	#[error("unsatisfiable range")]
	UnsatisfiableRange,
}

impl IntoResponse for FileStreamError {
	fn into_response(self) -> Response {
		match self {
			Self::Io(_) | Self::Runtime(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
			Self::InvalidRangeValue => StatusCode::BAD_REQUEST.into_response(),
			Self::UnsatisfiableRange => StatusCode::RANGE_NOT_SATISFIABLE.into_response(),
		}
	}
}

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

#[cfg(test)]
mod test {
	use http_body_util::BodyExt;

	use super::*;

	#[test]
	fn parse_ranges() {
		const FILE_SIZE: u64 = 10000;
		const END: u64 = FILE_SIZE - 1;

		// macro_rules! get_start {
		// 	() => {
		// 		(0, 0.to_string().into())
		// 	};
		// }
		//
		// macro_rules! get_end {
		// 	() => {
		// 		(END, END.to_string().into())
		// 	};
		// }

		macro_rules! get_field {
			($n:expr) => {
				($n, $n.to_string().into())
			};
		}

		macro_rules! get_range {
			($start:expr, $end:expr) => {
				RangeValue {
					start: get_field!($start),
					end: get_field!($end),
				}
			};
		}

		struct Case(&'static str, u64, bool, Vec<RangeValue>);

		let cases = [
			Case("bytes=200-", FILE_SIZE, false, vec![get_range!(200, END)]),
			Case("bytes=,200-,", FILE_SIZE, false, vec![get_range!(200, END)]),
			Case(
				"bytes=-500",
				FILE_SIZE,
				false,
				vec![get_range!(FILE_SIZE - 500, END)],
			),
			Case(
				"bytes=,-500,",
				FILE_SIZE,
				false,
				vec![get_range!(FILE_SIZE - 500, END)],
			),
			Case(
				"bytes=200-500",
				FILE_SIZE,
				false,
				vec![get_range!(200, 500)],
			),
			Case(
				"bytes=200-500, 800-1000, 5000-6000, -1000",
				FILE_SIZE,
				true,
				vec![
					get_range!(200, 500),
					get_range!(800, 1000),
					get_range!(5000, 6000),
					get_range!(FILE_SIZE - 1000, END),
				],
			),
			Case(
				"bytes=200-800, 500-1000, 5000-6000, -1000",
				FILE_SIZE,
				false,
				vec![
					get_range!(200, 1000),
					get_range!(5000, 6000),
					get_range!(FILE_SIZE - 1000, END),
				],
			),
			Case(
				"bytes=200-1000, 500-800, 5000-6000, -1000",
				FILE_SIZE,
				false,
				vec![
					get_range!(200, 1000),
					get_range!(5000, 6000),
					get_range!(FILE_SIZE - 1000, END),
				],
			),
			Case(
				"bytes=200-800, 500-5000, 1000-6000, -1000",
				FILE_SIZE,
				false,
				vec![get_range!(200, 6000), get_range!(FILE_SIZE - 1000, END)],
			),
			Case(
				"bytes=200-500, 800-1000, 5000-6000, -4500",
				FILE_SIZE,
				true,
				vec![
					get_range!(200, 500),
					get_range!(800, 1000),
					get_range!(5000, END),
				],
			),
			Case(
				"bytes=200-500, 800-1000, -1000, 5000-6000, -4500, -500",
				FILE_SIZE,
				true,
				vec![
					get_range!(200, 500),
					get_range!(800, 1000),
					get_range!(5000, END),
				],
			),
			Case(
				"bytes=200-500, 800-1000, 5000-6000, -9100",
				FILE_SIZE,
				true,
				vec![get_range!(200, 500), get_range!(800, END)],
			),
			Case(
				"bytes=200-500, 800-, 5000-6000, -500",
				FILE_SIZE,
				true,
				vec![get_range!(200, 500), get_range!(800, END)],
			),
			Case(
				"bytes=200-1000, 800-, 5000-6000, -500",
				FILE_SIZE,
				true,
				vec![get_range!(200, END)],
			),
			Case(
				"bytes=200-300, 800-, 5000-6000, -9500",
				FILE_SIZE,
				true,
				vec![get_range!(200, 300), get_range!(FILE_SIZE - 9500, END)],
			),
			Case(
				"bytes=200-1000, 800-, 5000-6000, -9500",
				FILE_SIZE,
				true,
				vec![get_range!(200, END)],
			),
			Case(
				"bytes=200-1000, 1100-2000, 5000-6000, -9500",
				FILE_SIZE,
				true,
				vec![get_range!(200, END)],
			),
			Case(
				"bytes=200-1000, 1100-2000, 5000-6000, -6500",
				FILE_SIZE,
				true,
				vec![get_range!(200, 2000), get_range!(FILE_SIZE - 6500, END)],
			),
			Case(
				"bytes=100-300, 500-1000, 1100-7000, 5000-6000, -1500, 7000-8000, -500",
				FILE_SIZE,
				true,
				vec![
					get_range!(100, 300),
					get_range!(500, 8000),
					get_range!(FILE_SIZE - 1500, END),
				],
			),
			Case(
				"bytes=100-300, 500-1000, 1100-7000, 5000-6000, -1500, 7000-8000, -500, -9500",
				FILE_SIZE,
				true,
				vec![get_range!(100, 300), get_range!(FILE_SIZE - 9500, END)],
			),
			Case(
				"bytes=100-300, 500-1000, 1100-7000, 5000-6000, -1500, 7000-8000, -500, -9800",
				FILE_SIZE,
				true,
				vec![get_range!(100, END)],
			),
			Case(
				"bytes=100-300, -0, 1100-7000, 7000-8000, 15000-20000",
				FILE_SIZE,
				true,
				vec![get_range!(100, 300), get_range!(1100, 8000)],
			),
			Case(
				"bytes=100-100, -0, 1000-7000, 8000-20000",
				FILE_SIZE,
				true,
				vec![
					get_range!(100, 100),
					get_range!(1000, 7000),
					get_range!(8000, END),
				],
			),
			Case(
				"bytes=100-100, -0, 1000-7000, -20000, 8000-20000",
				FILE_SIZE,
				true,
				vec![get_range!(0, END)],
			),
			Case(
				"bytes=-1000, 5000-6000, 800-1000, 200-500",
				FILE_SIZE,
				true,
				vec![
					get_range!(FILE_SIZE - 1000, END),
					get_range!(5000, 6000),
					get_range!(800, 1000),
					get_range!(200, 500),
				],
			),
			Case(
				"bytes=-1000, 5000-6000, 500-1000, 200-800",
				FILE_SIZE,
				true,
				vec![
					get_range!(FILE_SIZE - 1000, END),
					get_range!(5000, 6000),
					get_range!(200, 1000),
				],
			),
			Case(
				"bytes=-1000, 5000-6000, 500-800, 200-1000",
				FILE_SIZE,
				true,
				vec![
					get_range!(FILE_SIZE - 1000, END),
					get_range!(5000, 6000),
					get_range!(200, 1000),
				],
			),
			Case(
				"bytes=-1000, 1000-6000, 500-5000, 200-800",
				FILE_SIZE,
				true,
				vec![get_range!(FILE_SIZE - 1000, END), get_range!(200, 6000)],
			),
			Case(
				"bytes=-4500, 5000-6000, 800-1000, 200-500",
				FILE_SIZE,
				true,
				vec![
					get_range!(5000, END),
					get_range!(800, 1000),
					get_range!(200, 500),
				],
			),
			Case(
				"bytes=-500, -4500, 5000-6000, -1000, 800-1000, 200-500",
				FILE_SIZE,
				true,
				vec![
					get_range!(5000, END),
					get_range!(800, 1000),
					get_range!(200, 500),
				],
			),
			Case(
				"bytes=-9100, 5000-6000, 800-1000, 200-500",
				FILE_SIZE,
				true,
				vec![get_range!(800, END), get_range!(200, 500)],
			),
			Case(
				"bytes=-500, 5000-6000, 800-, 200-500",
				FILE_SIZE,
				true,
				vec![get_range!(800, END), get_range!(200, 500)],
			),
			Case(
				"bytes=-500, 5000-6000, 800-, 200-1000",
				FILE_SIZE,
				true,
				vec![get_range!(200, END)],
			),
			Case(
				"bytes=-9500, 5000-6000, 800-, 200-300",
				FILE_SIZE,
				true,
				vec![get_range!(FILE_SIZE - 9500, END), get_range!(200, 300)],
			),
			Case(
				"bytes=-9500, 5000-6000, 800-, 200-1000",
				FILE_SIZE,
				true,
				vec![get_range!(200, END)],
			),
			Case(
				"bytes=-9500, 5000-6000, 1100-2000, 200-1000",
				FILE_SIZE,
				true,
				vec![get_range!(200, END)],
			),
			Case(
				"bytes=-6500, 5000-6000, 1100-2000, 200-1000",
				FILE_SIZE,
				true,
				vec![get_range!(FILE_SIZE - 6500, END), get_range!(200, 2000)],
			),
			Case(
				"bytes=-500, 7000-8000, -1500, 5000-6000, 1100-7000, 500-1000, 100-300",
				FILE_SIZE,
				true,
				vec![
					get_range!(FILE_SIZE - 1500, END),
					get_range!(500, 8000),
					get_range!(100, 300),
				],
			),
			Case(
				"bytes=-9500, -500, 7000-8000, -1500, 5000-6000, 1100-7000, 500-1000, 100-300",
				FILE_SIZE,
				true,
				vec![get_range!(FILE_SIZE - 9500, END), get_range!(100, 300)],
			),
			Case(
				"bytes=-9800, -500, 7000-8000, -1500, 5000-6000, 1100-7000, 500-1000, 100-300",
				FILE_SIZE,
				true,
				vec![get_range!(100, END)],
			),
			Case(
				"bytes=15000-20000, 7000-8000, 1100-7000, -0, 100-300",
				FILE_SIZE,
				true,
				vec![get_range!(1100, 8000), get_range!(100, 300)],
			),
			Case(
				"bytes=8000-20000, 1000-7000, -0, 100-100",
				FILE_SIZE,
				true,
				vec![
					get_range!(8000, END),
					get_range!(1000, 7000),
					get_range!(100, 100),
				],
			),
			Case(
				"bytes=8000-20000, -20000, 1000-7000, -0, 100-100",
				FILE_SIZE,
				true,
				vec![get_range!(0, END)],
			),
		];

		for case in cases {
			let ranges = parse_range_header_value(case.0, case.1, case.2).unwrap();
			assert_eq!(ranges, case.3);
		}

		// ----------

		struct ErrorCase(&'static str, u64, bool);

		let cases = [
			ErrorCase("bytes=500-200", FILE_SIZE, false),
			ErrorCase(
				"200-800, 500-5000, 1000-6000, 400-7000, -1000",
				FILE_SIZE,
				true,
			),
			ErrorCase("200-800, 100-1000, 500-4000", FILE_SIZE, false),
			ErrorCase("200-800, 1000-2000, 500-4000", FILE_SIZE, false),
			ErrorCase("200-800, 1000-2000, 500-4000", FILE_SIZE, true),
			ErrorCase("4000-8000, 1000-2000, 3000-3500", FILE_SIZE, true),
		];

		for case in cases {
			let result = parse_range_header_value(case.0, case.1, case.2);
			assert!(result.is_err());
		}
	}

	// --------------------------------------------------

	use crate::common::Deferred;

	const FILE_SIZE: usize = 32 * 1024;

	#[tokio::test]
	async fn single_range_streaming() {
		const FILE: &str = "test_single_range";

		let mut contents = Vec::<u8>::with_capacity(FILE_SIZE);
		for i in 0..FILE_SIZE {
			contents.push({ i % 7 } as u8);
		}

		std::fs::write(FILE, &contents).unwrap();

		let _deferred = Deferred::call(|| std::fs::remove_file(FILE).unwrap());

		let gzip_content_encoding_value = HeaderValue::from_static("gzip");
		let content_type_value = HeaderValue::from_static(mime::TEXT_PLAIN_UTF_8.as_ref());
		let file_size_string = &FILE_SIZE.to_string();

		// -------------------------

		let response = FileStream::open(FILE).await.unwrap().into_response();

		assert_eq!(StatusCode::OK, response.status());
		assert!(response.headers().get(CONTENT_TYPE).is_none());
		assert!(response.headers().get(CONTENT_DISPOSITION).is_none());
		assert!(response.headers().get(ACCEPT_RANGES).is_none());
		assert_eq!(
			file_size_string,
			response.headers().get(CONTENT_LENGTH).unwrap()
		);

		let body = response.into_body().collect().await.unwrap().to_bytes();
		assert_eq!(FILE_SIZE, body.len());
		assert_eq!(contents, Into::<Vec<u8>>::into(body));

		// -------------------------

		let mut file_stream = FileStream::open(FILE).await.unwrap();
		file_stream.configure([_as_attachment(), _content_type(content_type_value.clone())]);

		let response = file_stream.into_response();

		assert_eq!(StatusCode::OK, response.status());
		assert!(response.headers().get(CONTENT_ENCODING).is_none());
		assert_eq!(
			mime::TEXT_PLAIN_UTF_8.as_ref(),
			response
				.headers()
				.get(CONTENT_TYPE)
				.unwrap()
				.to_str()
				.unwrap()
		);

		assert_eq!(
			"attachment",
			response
				.headers()
				.get(CONTENT_DISPOSITION)
				.unwrap()
				.to_str()
				.unwrap()
		);

		assert!(response.headers().get(ACCEPT_RANGES).is_none());
		assert_eq!(
			file_size_string,
			response.headers().get(CONTENT_LENGTH).unwrap()
		);

		let body = response.into_body().collect().await.unwrap().to_bytes();
		assert_eq!(FILE_SIZE, body.len());
		assert_eq!(contents, Into::<Vec<u8>>::into(body));

		// -------------------------

		let mut file_stream = FileStream::open(FILE).await.unwrap();
		file_stream.configure([
			_as_attachment(),
			_content_encoding(gzip_content_encoding_value.clone()),
			_content_type(content_type_value.clone()),
			_file_name("test".into()),
		]);

		let response = file_stream.into_response();

		assert_eq!(StatusCode::OK, response.status());
		assert_eq!(
			"gzip",
			response
				.headers()
				.get(CONTENT_ENCODING)
				.unwrap()
				.to_str()
				.unwrap()
		);

		assert_eq!(
			mime::TEXT_PLAIN_UTF_8.as_ref(),
			response
				.headers()
				.get(CONTENT_TYPE)
				.unwrap()
				.to_str()
				.unwrap()
		);

		assert_eq!(
			"attachment; filename=\"test\"",
			response
				.headers()
				.get(CONTENT_DISPOSITION)
				.unwrap()
				.to_str()
				.unwrap()
		);

		assert!(response.headers().get(ACCEPT_RANGES).is_none());
		assert_eq!(
			file_size_string,
			response.headers().get(CONTENT_LENGTH).unwrap()
		);

		let body = response.into_body().collect().await.unwrap().to_bytes();
		assert_eq!(FILE_SIZE, body.len());
		assert_eq!(contents, Into::<Vec<u8>>::into(body));

		// -------------------------

		let mut file_stream = FileStream::open(FILE).await.unwrap();
		file_stream.configure(_file_name("test-Ω".into()));

		let response = file_stream.into_response();

		assert_eq!(StatusCode::OK, response.status());
		assert!(response.headers().get(CONTENT_ENCODING).is_none());
		assert!(response.headers().get(CONTENT_TYPE).is_none());
		assert_eq!(
			"inline; filename*=utf-8''test%2D%CE%A9",
			response
				.headers()
				.get(CONTENT_DISPOSITION)
				.unwrap()
				.to_str()
				.unwrap()
		);

		assert!(response.headers().get(ACCEPT_RANGES).is_none());
		assert_eq!(
			file_size_string,
			response.headers().get(CONTENT_LENGTH).unwrap()
		);

		let body = response.into_body().collect().await.unwrap().to_bytes();
		assert_eq!(FILE_SIZE, body.len());
		assert_eq!(contents, Into::<Vec<u8>>::into(body));

		// -------------------------

		let mut file_stream = FileStream::open(FILE).await.unwrap();
		file_stream.configure([
			_content_type(content_type_value.clone()),
			_to_support_partial_content(),
			_content_encoding(gzip_content_encoding_value.clone()),
		]);

		let response = file_stream.into_response();

		assert_eq!(StatusCode::OK, response.status());
		assert_eq!(
			"gzip",
			response
				.headers()
				.get(CONTENT_ENCODING)
				.unwrap()
				.to_str()
				.unwrap()
		);

		assert_eq!(
			mime::TEXT_PLAIN_UTF_8.as_ref(),
			response
				.headers()
				.get(CONTENT_TYPE)
				.unwrap()
				.to_str()
				.unwrap()
		);

		assert!(response.headers().get(CONTENT_DISPOSITION).is_none());
		assert_eq!(
			"bytes",
			response
				.headers()
				.get(ACCEPT_RANGES)
				.unwrap()
				.to_str()
				.unwrap()
		);

		assert_eq!(
			file_size_string,
			response.headers().get(CONTENT_LENGTH).unwrap()
		);

		let body = response.into_body().collect().await.unwrap().to_bytes();
		assert_eq!(FILE_SIZE, body.len());
		assert_eq!(contents, Into::<Vec<u8>>::into(body));

		// -------------------------

		let mut file_stream = FileStream::open_ranges(FILE, "bytes=1024-16383", false)
			.await
			.unwrap();
		file_stream.configure([
			_content_type(content_type_value.clone()),
			_content_encoding(gzip_content_encoding_value.clone()),
		]);

		let response = file_stream.into_response();

		assert_eq!(StatusCode::PARTIAL_CONTENT, response.status());
		assert_eq!(
			"gzip",
			response
				.headers()
				.get(CONTENT_ENCODING)
				.unwrap()
				.to_str()
				.unwrap()
		);

		assert_eq!(
			mime::TEXT_PLAIN_UTF_8.as_ref(),
			response
				.headers()
				.get(CONTENT_TYPE)
				.unwrap()
				.to_str()
				.unwrap()
		);

		assert_eq!(
			"bytes 1024-16383/32768",
			response
				.headers()
				.get(CONTENT_RANGE)
				.unwrap()
				.to_str()
				.unwrap()
		);

		assert!(response.headers().get(CONTENT_DISPOSITION).is_none());
		assert!(response.headers().get(ACCEPT_RANGES).is_none());

		assert_eq!("15360", response.headers().get(CONTENT_LENGTH).unwrap());

		let body = response.into_body().collect().await.unwrap().to_bytes();
		assert_eq!(15360, body.len());

		let mut range_content = Vec::with_capacity(50);
		for i in 1024..16384 {
			range_content.push({ i % 7 } as u8);
		}

		assert_eq!(range_content, Into::<Vec<u8>>::into(body));

		// -------------------------
		// Gzip

		let mut file_stream = FileStream::open_with_encoding(FILE, ContentCoding::Gzip(6))
			.await
			.unwrap();

		file_stream.configure([
			_content_type(content_type_value.clone()),
			_content_encoding(gzip_content_encoding_value.clone()),
		]);

		let response = file_stream.into_response();

		assert_eq!(StatusCode::OK, response.status());
		assert_eq!(
			mime::TEXT_PLAIN_UTF_8.as_ref(),
			response
				.headers()
				.get(CONTENT_TYPE)
				.unwrap()
				.to_str()
				.unwrap()
		);

		assert_eq!(
			"gzip",
			response
				.headers()
				.get(CONTENT_ENCODING)
				.unwrap()
				.to_str()
				.unwrap()
		);

		assert_eq!(
			"none",
			response
				.headers()
				.get(ACCEPT_RANGES)
				.unwrap()
				.to_str()
				.unwrap()
		);

		assert!(response.headers().get(CONTENT_DISPOSITION).is_none());
		assert!(response.headers().get(CONTENT_LENGTH).is_none());

		let body = response.into_body().collect().await.unwrap().to_bytes();
		assert!(FILE_SIZE > body.len());
		assert_ne!(contents, Into::<Vec<u8>>::into(body));

		// -------------------------
		// Deflate

		let mut file_stream = FileStream::open_with_encoding(FILE, ContentCoding::Deflate(6))
			.await
			.unwrap();

		file_stream.configure([
			_content_type(content_type_value.clone()),
			_content_encoding(HeaderValue::from_static("deflate")),
		]);

		let response = file_stream.into_response();

		assert_eq!(StatusCode::OK, response.status());
		assert_eq!(
			mime::TEXT_PLAIN_UTF_8.as_ref(),
			response
				.headers()
				.get(CONTENT_TYPE)
				.unwrap()
				.to_str()
				.unwrap()
		);

		assert_eq!(
			"deflate",
			response
				.headers()
				.get(CONTENT_ENCODING)
				.unwrap()
				.to_str()
				.unwrap()
		);

		assert_eq!(
			"none",
			response
				.headers()
				.get(ACCEPT_RANGES)
				.unwrap()
				.to_str()
				.unwrap()
		);

		assert!(response.headers().get(CONTENT_DISPOSITION).is_none());
		assert!(response.headers().get(CONTENT_LENGTH).is_none());

		let body = response.into_body().collect().await.unwrap().to_bytes();
		assert!(FILE_SIZE > body.len());
		assert_ne!(contents, Into::<Vec<u8>>::into(body));

		// -------------------------
		// Brotli

		let mut file_stream = FileStream::open_with_encoding(FILE, ContentCoding::Brotli(6))
			.await
			.unwrap();

		file_stream.configure([
			_content_type(content_type_value.clone()),
			_content_encoding(HeaderValue::from_static("br")),
		]);

		let response = file_stream.into_response();

		assert_eq!(StatusCode::OK, response.status());
		assert_eq!(
			mime::TEXT_PLAIN_UTF_8.as_ref(),
			response
				.headers()
				.get(CONTENT_TYPE)
				.unwrap()
				.to_str()
				.unwrap()
		);

		assert_eq!(
			"br",
			response
				.headers()
				.get(CONTENT_ENCODING)
				.unwrap()
				.to_str()
				.unwrap()
		);

		assert_eq!(
			"none",
			response
				.headers()
				.get(ACCEPT_RANGES)
				.unwrap()
				.to_str()
				.unwrap()
		);

		assert!(response.headers().get(CONTENT_DISPOSITION).is_none());
		assert!(response.headers().get(CONTENT_LENGTH).is_none());

		let body = response.into_body().collect().await.unwrap().to_bytes();
		assert!(FILE_SIZE > body.len());
		assert_ne!(contents, Into::<Vec<u8>>::into(body));
	}

	#[test]
	fn part_header_size() {
		let boundary = "boundary";

		// |     |       |
		// \r\n--boundary
		let boundary_len = 4 + boundary.len();

		// |   |           | |    |
		// \r\nContent-Type: value
		let content_type_start_len = 2 + CONTENT_TYPE.as_str().len() + 2;

		// |   |            |       |
		// \r\nContent-Range: bytes
		let content_range_start_len = 2 + CONTENT_RANGE.as_str().len() + 8;

		// |       |
		// \r\n\r\n
		let end_line_len = 4;

		let content_type_value = HeaderValue::from_static(mime::TEXT_PLAIN_UTF_8.as_ref());

		// -------------------------

		let part_header_size_len = boundary_len
			+ content_type_start_len
			+ content_type_value.len()
			+ content_range_start_len
			+ "15-50/256".len()
			+ end_line_len;

		let range_value = RangeValue::new(15, 50);

		assert_eq!(
			part_header_size_len,
			super::part_header_size(
				boundary.len(),
				Some(&content_type_value),
				Some(&range_value),
				"256"
			)
		);

		// ----------

		let part_header_size_len =
			boundary_len + content_range_start_len + "150-5000/25600".len() + end_line_len;

		let range_value = RangeValue::new(150, 5000);

		assert_eq!(
			part_header_size_len,
			super::part_header_size(boundary.len(), None, Some(&range_value), "25600")
		);

		// ----------

		// | |            ||            |       |
		// 0-file_size len/file size len\r\n\r\n
		let range_len = 2 + "256".len() + 1 + "256".len();

		let part_header_size_len = boundary_len
			+ content_type_start_len
			+ content_type_value.len()
			+ content_range_start_len
			+ range_len
			+ end_line_len;

		assert_eq!(
			part_header_size_len,
			super::part_header_size(boundary.len(), Some(&content_type_value), None, "256")
		);
	}

	#[tokio::test]
	async fn stream_multipart_ranges() {
		const FILE: &str = "test_multipart";

		let mut contents = Vec::with_capacity(FILE_SIZE);
		for i in 0..FILE_SIZE {
			contents.push({ i % 7 } as u8);
		}

		std::fs::write(FILE, &contents).unwrap();

		let _deferred = Deferred::call(|| std::fs::remove_file(FILE).unwrap());

		let file_size_string = &FILE_SIZE.to_string();

		let content_encoding_value = HeaderValue::from_static("gzip");
		let content_type_value = HeaderValue::from_static(mime::TEXT_PLAIN_UTF_8.as_ref());
		let boundary_value = "boundary";

		//                 |     |       |     |
		//                 \r\n--boundary--\r\n
		let end_line_len = 4 + boundary_value.len() + 4;

		// -------------------------

		let mut file_stream = FileStream::open(FILE).await.unwrap();
		file_stream.configure([
			_boundary("boundary".into()),
			_content_encoding(content_encoding_value.clone()),
		]);

		let response = file_stream.into_response();
		assert_eq!(StatusCode::PARTIAL_CONTENT, response.status());
		assert_eq!(
			"gzip",
			response
				.headers()
				.get(CONTENT_ENCODING)
				.unwrap()
				.to_str()
				.unwrap()
		);

		assert_eq!(
			"multipart/byteranges; boundary=boundary",
			response
				.headers()
				.get(CONTENT_TYPE)
				.unwrap()
				.to_str()
				.unwrap()
		);

		let part_header_size =
			super::part_header_size(boundary_value.len(), None, None, file_size_string);
		let content_length = part_header_size + FILE_SIZE + end_line_len;
		dbg!(content_length);
		assert_eq!(
			content_length.to_string(),
			response
				.headers()
				.get(CONTENT_LENGTH)
				.unwrap()
				.to_str()
				.unwrap()
		);

		let body = &response.into_body().collect().await.unwrap().to_bytes()[part_header_size..];
		assert_eq!(contents, body[..body.len() - end_line_len]);

		// -------------------------

		let mut file_stream = FileStream::open(FILE).await.unwrap();
		file_stream.configure([
			_content_type(content_type_value.clone()),
			_boundary(boundary_value.into()),
		]);

		let response = file_stream.into_response();
		assert_eq!(StatusCode::PARTIAL_CONTENT, response.status());
		assert_eq!(
			"multipart/byteranges; boundary=boundary",
			response
				.headers()
				.get(CONTENT_TYPE)
				.unwrap()
				.to_str()
				.unwrap()
		);

		let part_header_size = super::part_header_size(
			boundary_value.len(),
			Some(&content_type_value),
			None,
			file_size_string,
		);

		let content_length = part_header_size + FILE_SIZE + end_line_len;
		dbg!(content_length);
		assert_eq!(
			content_length.to_string(),
			response
				.headers()
				.get(CONTENT_LENGTH)
				.unwrap()
				.to_str()
				.unwrap()
		);

		let body = &response.into_body().collect().await.unwrap().to_bytes()[part_header_size..];
		assert_eq!(contents, body[..body.len() - end_line_len]);

		// -------------------------

		let mut file_stream = FileStream::open_ranges(FILE, "bytes=12-24", false)
			.await
			.unwrap();
		file_stream.configure([
			_content_type(content_type_value.clone()),
			_content_encoding(content_encoding_value.clone()),
			_boundary(boundary_value.into()),
		]);

		let response = file_stream.into_response();
		assert_eq!(StatusCode::PARTIAL_CONTENT, response.status());
		assert_eq!(
			"gzip",
			response
				.headers()
				.get(CONTENT_ENCODING)
				.unwrap()
				.to_str()
				.unwrap()
		);

		assert_eq!(
			"multipart/byteranges; boundary=boundary",
			response
				.headers()
				.get(CONTENT_TYPE)
				.unwrap()
				.to_str()
				.unwrap()
		);

		let range = RangeValue::new(12, 24);

		let part_header_size = super::part_header_size(
			boundary_value.len(),
			Some(&content_type_value),
			Some(&range),
			file_size_string,
		);

		let content_length = part_header_size + range.size() as usize + end_line_len;
		dbg!(content_length);
		assert_eq!(
			content_length.to_string(),
			response
				.headers()
				.get(CONTENT_LENGTH)
				.unwrap()
				.to_str()
				.unwrap()
		);

		let body = &response.into_body().collect().await.unwrap().to_bytes()[part_header_size..];
		assert_eq!(contents[12..=24], body[..body.len() - end_line_len]);

		// -------------------------

		let mut file_stream = FileStream::open_ranges(FILE, "bytes=12-24, 1024-4095", false)
			.await
			.unwrap();

		file_stream.configure([
			_content_type(content_type_value.clone()),
			_content_encoding(content_encoding_value.clone()),
			_boundary(boundary_value.into()),
		]);

		let response = file_stream.into_response();
		assert_eq!(StatusCode::PARTIAL_CONTENT, response.status());
		assert_eq!(
			"gzip",
			response
				.headers()
				.get(CONTENT_ENCODING)
				.unwrap()
				.to_str()
				.unwrap()
		);

		assert_eq!(
			"multipart/byteranges; boundary=boundary",
			response
				.headers()
				.get(CONTENT_TYPE)
				.unwrap()
				.to_str()
				.unwrap()
		);

		let range_1 = RangeValue::new(12, 24);

		let part_1_header_size = super::part_header_size(
			boundary_value.len(),
			Some(&content_type_value),
			Some(&range_1),
			file_size_string,
		);

		let range_2 = RangeValue::new(1024, 4095);

		let part_2_header_size = super::part_header_size(
			boundary_value.len(),
			Some(&content_type_value),
			Some(&range_2),
			file_size_string,
		);

		let content_length = part_1_header_size
			+ range_1.size() as usize
			+ part_2_header_size
			+ range_2.size() as usize
			+ end_line_len;

		dbg!(content_length);
		assert_eq!(
			content_length.to_string(),
			response
				.headers()
				.get(CONTENT_LENGTH)
				.unwrap()
				.to_str()
				.unwrap()
		);

		let full_body = response.into_body().collect().await.unwrap().to_bytes();

		let body_1 = &full_body[part_1_header_size
			..full_body.len() - (part_2_header_size + range_2.size() as usize + end_line_len)];
		assert_eq!(&contents[12..=24], body_1);

		let body_2 = &full_body[part_1_header_size + range_1.size() as usize + part_2_header_size
			..full_body.len() - end_line_len];
		assert_eq!(&contents[1024..=4095], body_2);

		// -------------------------

		let mut file_stream = FileStream::open_ranges(FILE, "bytes=1024-4095, 12-24", true)
			.await
			.unwrap();
		file_stream.configure([
			_content_type(content_type_value.clone()),
			_content_encoding(content_encoding_value.clone()),
			_boundary(boundary_value.into()),
		]);

		let response = file_stream.into_response();
		assert_eq!(StatusCode::PARTIAL_CONTENT, response.status());
		assert_eq!(
			"gzip",
			response
				.headers()
				.get(CONTENT_ENCODING)
				.unwrap()
				.to_str()
				.unwrap()
		);

		assert_eq!(
			"multipart/byteranges; boundary=boundary",
			response
				.headers()
				.get(CONTENT_TYPE)
				.unwrap()
				.to_str()
				.unwrap()
		);

		let range_1 = RangeValue::new(1024, 4095);

		let part_1_header_size = super::part_header_size(
			boundary_value.len(),
			Some(&content_type_value),
			Some(&range_1),
			file_size_string,
		);

		let range_2 = RangeValue::new(12, 24);

		let part_2_header_size = super::part_header_size(
			boundary_value.len(),
			Some(&content_type_value),
			Some(&range_2),
			file_size_string,
		);

		let content_length = part_1_header_size
			+ range_1.size() as usize
			+ part_2_header_size
			+ range_2.size() as usize
			+ end_line_len;

		dbg!(content_length);
		assert_eq!(
			content_length.to_string(),
			response
				.headers()
				.get(CONTENT_LENGTH)
				.unwrap()
				.to_str()
				.unwrap()
		);

		let full_body = response.into_body().collect().await.unwrap().to_bytes();

		let body_1 = &full_body[part_1_header_size
			..full_body.len() - (part_2_header_size + range_2.size() as usize + end_line_len)];
		assert_eq!(&contents[1024..=4095], body_1);

		let body_2 = &full_body[part_1_header_size + range_1.size() as usize + part_2_header_size
			..full_body.len() - end_line_len];
		assert_eq!(&contents[12..=24], body_2);

		// -------------------------

		let mut file_stream = FileStream::open_ranges(FILE, "bytes=12-2048, 1024-4095, -512", false)
			.await
			.unwrap();

		file_stream.configure([
			_content_type(content_type_value.clone()),
			_boundary(boundary_value.into()),
		]);

		let response = file_stream.into_response();
		assert_eq!(StatusCode::PARTIAL_CONTENT, response.status());
		assert_eq!(
			"multipart/byteranges; boundary=boundary",
			response
				.headers()
				.get(CONTENT_TYPE)
				.unwrap()
				.to_str()
				.unwrap()
		);

		let range_1 = RangeValue::new(12, 4095);

		let part_1_header_size = super::part_header_size(
			boundary_value.len(),
			Some(&content_type_value),
			Some(&range_1),
			file_size_string,
		);

		let range_2 = RangeValue::new(FILE_SIZE as u64 - 512, FILE_SIZE as u64 - 1);

		let part_2_header_size = super::part_header_size(
			boundary_value.len(),
			Some(&content_type_value),
			Some(&range_2),
			file_size_string,
		);

		let content_length = part_1_header_size
			+ range_1.size() as usize
			+ part_2_header_size
			+ range_2.size() as usize
			+ end_line_len;

		dbg!(content_length);
		assert_eq!(
			content_length.to_string(),
			response
				.headers()
				.get(CONTENT_LENGTH)
				.unwrap()
				.to_str()
				.unwrap()
		);

		let full_body = response.into_body().collect().await.unwrap().to_bytes();

		let body_1 = &full_body[part_1_header_size
			..full_body.len() - (part_2_header_size + range_2.size() as usize + end_line_len)];
		assert_eq!(&contents[12..=4095], body_1);

		let body_2 = &full_body[part_1_header_size + range_1.size() as usize + part_2_header_size
			..full_body.len() - end_line_len];
		assert_eq!(
			&contents[range_2.start() as usize..=range_2.end() as usize],
			body_2
		);

		// -------------------------

		let mut file_stream = FileStream::open_ranges(FILE, "bytes=1024-4095, 12-2048, -512", true)
			.await
			.unwrap();

		file_stream.configure([
			_content_type(content_type_value.clone()),
			_boundary(boundary_value.into()),
		]);

		let response = file_stream.into_response();
		assert_eq!(StatusCode::PARTIAL_CONTENT, response.status());
		assert_eq!(
			"multipart/byteranges; boundary=boundary",
			response
				.headers()
				.get(CONTENT_TYPE)
				.unwrap()
				.to_str()
				.unwrap()
		);

		let range_1 = RangeValue::new(FILE_SIZE as u64 - 512, FILE_SIZE as u64 - 1);

		let part_1_header_size = super::part_header_size(
			boundary_value.len(),
			Some(&content_type_value),
			Some(&range_1),
			file_size_string,
		);

		let range_2 = RangeValue::new(12, 4095);

		let part_2_header_size = super::part_header_size(
			boundary_value.len(),
			Some(&content_type_value),
			Some(&range_2),
			file_size_string,
		);

		let content_length = part_1_header_size
			+ range_1.size() as usize
			+ part_2_header_size
			+ range_2.size() as usize
			+ end_line_len;

		dbg!(content_length);
		assert_eq!(
			content_length.to_string(),
			response
				.headers()
				.get(CONTENT_LENGTH)
				.unwrap()
				.to_str()
				.unwrap()
		);

		let full_body = response.into_body().collect().await.unwrap().to_bytes();

		let body_1 = &full_body[part_1_header_size
			..full_body.len() - (part_2_header_size + range_2.size() as usize + end_line_len)];
		assert_eq!(
			&contents[range_1.start() as usize..=range_1.end() as usize],
			body_1
		);

		let body_2 = &full_body[part_1_header_size + range_1.size() as usize + part_2_header_size
			..full_body.len() - end_line_len];
		assert_eq!(&contents[12..=4095], body_2);

		// -------------------------

		let mut file_stream = FileStream::open_ranges(
			FILE,
			"bytes=100-100, 500-1000, 1100-7000, 5000-6000, -1500, 7000-8000, -500",
			true,
		)
		.await
		.unwrap();

		file_stream.configure([
			_content_type(content_type_value.clone()),
			_boundary(boundary_value.into()),
		]);

		let response = file_stream.into_response();
		assert_eq!(StatusCode::PARTIAL_CONTENT, response.status());
		assert_eq!(
			"multipart/byteranges; boundary=boundary",
			response
				.headers()
				.get(CONTENT_TYPE)
				.unwrap()
				.to_str()
				.unwrap()
		);

		let range_1 = RangeValue::new(100, 100);

		let part_1_header_size = super::part_header_size(
			boundary_value.len(),
			Some(&content_type_value),
			Some(&range_1),
			file_size_string,
		);

		let range_2 = RangeValue::new(500, 8000);

		let part_2_header_size = super::part_header_size(
			boundary_value.len(),
			Some(&content_type_value),
			Some(&range_2),
			file_size_string,
		);

		let range_3 = RangeValue::new(FILE_SIZE as u64 - 1500, FILE_SIZE as u64 - 1);

		let part_3_header_size = super::part_header_size(
			boundary_value.len(),
			Some(&content_type_value),
			Some(&range_3),
			file_size_string,
		);

		let content_length = part_1_header_size
			+ range_1.size() as usize
			+ part_2_header_size
			+ range_2.size() as usize
			+ part_3_header_size
			+ range_3.size() as usize
			+ end_line_len;

		dbg!(content_length);
		assert_eq!(
			content_length.to_string(),
			response
				.headers()
				.get(CONTENT_LENGTH)
				.unwrap()
				.to_str()
				.unwrap()
		);

		let full_body = response.into_body().collect().await.unwrap().to_bytes();

		let body_1 = &full_body[part_1_header_size
			..full_body.len()
				- (part_2_header_size
					+ range_2.size() as usize
					+ part_3_header_size
					+ range_3.size() as usize
					+ end_line_len)];
		assert_eq!(
			&contents[range_1.start() as usize..=range_1.end() as usize],
			body_1
		);

		let body_2 = &full_body[part_1_header_size + range_1.size() as usize + part_2_header_size
			..full_body.len() - (part_3_header_size + range_3.size() as usize + end_line_len)];
		assert_eq!(
			&contents[range_2.start() as usize..=range_2.end() as usize],
			body_2
		);

		let body_3 = &full_body[part_1_header_size
			+ range_1.size() as usize
			+ part_2_header_size
			+ range_2.size() as usize
			+ part_3_header_size..full_body.len() - end_line_len];
		assert_eq!(
			&contents[range_3.start() as usize..=range_3.end() as usize],
			body_3
		);

		// -------------------------

		let mut file_stream = FileStream::open_ranges(
			FILE,
			"bytes=-500, 7000-8000, -1500, 5000-6000, 1100-7000, 500-1000, 100-100",
			true,
		)
		.await
		.unwrap();

		file_stream.configure([
			_content_type(content_type_value.clone()),
			_boundary(boundary_value.into()),
		]);

		let response = file_stream.into_response();
		assert_eq!(StatusCode::PARTIAL_CONTENT, response.status());
		assert_eq!(
			"multipart/byteranges; boundary=boundary",
			response
				.headers()
				.get(CONTENT_TYPE)
				.unwrap()
				.to_str()
				.unwrap()
		);

		let range_1 = RangeValue::new(FILE_SIZE as u64 - 1500, FILE_SIZE as u64 - 1);

		let part_1_header_size = super::part_header_size(
			boundary_value.len(),
			Some(&content_type_value),
			Some(&range_1),
			file_size_string,
		);

		let range_2 = RangeValue::new(500, 8000);

		let part_2_header_size = super::part_header_size(
			boundary_value.len(),
			Some(&content_type_value),
			Some(&range_2),
			file_size_string,
		);

		let range_3 = RangeValue::new(100, 100);

		let part_3_header_size = super::part_header_size(
			boundary_value.len(),
			Some(&content_type_value),
			Some(&range_3),
			file_size_string,
		);

		let content_length = part_1_header_size
			+ range_1.size() as usize
			+ part_2_header_size
			+ range_2.size() as usize
			+ part_3_header_size
			+ range_3.size() as usize
			+ end_line_len;

		dbg!(content_length);
		assert_eq!(
			content_length.to_string(),
			response
				.headers()
				.get(CONTENT_LENGTH)
				.unwrap()
				.to_str()
				.unwrap()
		);

		let full_body = response.into_body().collect().await.unwrap().to_bytes();

		let body_1 = &full_body[part_1_header_size
			..full_body.len()
				- (part_2_header_size
					+ range_2.size() as usize
					+ part_3_header_size
					+ range_3.size() as usize
					+ end_line_len)];
		assert_eq!(
			&contents[range_1.start() as usize..=range_1.end() as usize],
			body_1
		);

		let body_2 = &full_body[part_1_header_size + range_1.size() as usize + part_2_header_size
			..full_body.len() - (part_3_header_size + range_3.size() as usize + end_line_len)];
		assert_eq!(
			&contents[range_2.start() as usize..=range_2.end() as usize],
			body_2
		);

		let body_3 = &full_body[part_1_header_size
			+ range_1.size() as usize
			+ part_2_header_size
			+ range_2.size() as usize
			+ part_3_header_size..full_body.len() - end_line_len];
		assert_eq!(
			&contents[range_3.start() as usize..=range_3.end() as usize],
			body_3
		);

		// -------------------------

		let mut file_stream = FileStream::open_ranges(
			FILE,
			"bytes=10-11, -0, 1000-7000, -40000, 8000-40000",
			false,
		)
		.await
		.unwrap();

		file_stream.configure(_boundary(boundary_value.into()));

		let response = file_stream.into_response();
		assert_eq!(StatusCode::PARTIAL_CONTENT, response.status());
		assert_eq!(
			"multipart/byteranges; boundary=boundary",
			response
				.headers()
				.get(CONTENT_TYPE)
				.unwrap()
				.to_str()
				.unwrap()
		);

		let range = RangeValue::new(0, FILE_SIZE as u64 - 1);

		let part_header_size =
			super::part_header_size(boundary_value.len(), None, Some(&range), file_size_string);

		let content_length = part_header_size + range.size() as usize + end_line_len;

		dbg!(content_length);
		assert_eq!(
			content_length.to_string(),
			response
				.headers()
				.get(CONTENT_LENGTH)
				.unwrap()
				.to_str()
				.unwrap()
		);

		let full_body = response.into_body().collect().await.unwrap().to_bytes();

		let body = &full_body[part_header_size..full_body.len() - end_line_len];
		assert_eq!(
			&contents[range.start() as usize..=range.end() as usize],
			body
		);

		// -------------------------

		let mut file_stream =
			FileStream::open_ranges(FILE, "bytes=8000-40000, -40000, 1000-7000, -0, 10-11", true)
				.await
				.unwrap();

		file_stream.configure(_boundary(boundary_value.into()));

		let response = file_stream.into_response();
		assert_eq!(StatusCode::PARTIAL_CONTENT, response.status());
		assert_eq!(
			"multipart/byteranges; boundary=boundary",
			response
				.headers()
				.get(CONTENT_TYPE)
				.unwrap()
				.to_str()
				.unwrap()
		);

		let range = RangeValue::new(0, FILE_SIZE as u64 - 1);

		let part_header_size =
			super::part_header_size(boundary_value.len(), None, Some(&range), file_size_string);

		let content_length = part_header_size + range.size() as usize + end_line_len;

		dbg!(content_length);
		assert_eq!(
			content_length.to_string(),
			response
				.headers()
				.get(CONTENT_LENGTH)
				.unwrap()
				.to_str()
				.unwrap()
		);

		let full_body = response.into_body().collect().await.unwrap().to_bytes();

		let body = &full_body[part_header_size..full_body.len() - end_line_len];
		assert_eq!(
			&contents[range.start() as usize..=range.end() as usize],
			body
		);
	}

	// --------------------------------------------------

	#[tokio::test]
	#[should_panic = "applied dynamic encoding `gzip` and Content-Encoding `br` are different"]
	async fn unmatching_encoding() {
		const FILE: &str = "panic-1";

		let _ = std::fs::File::create(FILE).unwrap();
		let _deferred = Deferred::call(|| std::fs::remove_file(FILE).unwrap());

		let mut file_stream = FileStream::open_with_encoding(FILE, ContentCoding::Gzip(6))
			.await
			.unwrap();
		file_stream.configure(_content_encoding(HeaderValue::from_static("br")));
	}

	#[tokio::test]
	#[should_panic = "cannot support partial content with dynamic encoding"]
	async fn support_partial_content_with_dynamic_encoding() {
		const FILE: &str = "panic-2";

		let _ = std::fs::File::create(FILE).unwrap();
		let _deferred = Deferred::call(|| std::fs::remove_file(FILE).unwrap());

		let mut file_stream = FileStream::open_with_encoding(FILE, ContentCoding::Gzip(6))
			.await
			.unwrap();
		file_stream.configure(_to_support_partial_content());
	}

	#[tokio::test]
	#[should_panic = "boundary cannot be set with dynamic encoding"]
	async fn boundary_with_dynamic_encoding() {
		const FILE: &str = "panic-3";

		let _ = std::fs::File::create(FILE).unwrap();
		let _deferred = Deferred::call(|| std::fs::remove_file(FILE).unwrap());

		let mut file_stream = FileStream::open_with_encoding(FILE, ContentCoding::Gzip(6))
			.await
			.unwrap();
		file_stream.configure(_boundary("boundary".into()));
	}

	#[tokio::test]
	#[should_panic = "boundary exceeds 70 characters"]
	async fn boundary_exceeds_character_limit() {
		const FILE: &str = "panic-4";

		let _ = std::fs::File::create(FILE).unwrap();
		let _deferred = Deferred::call(|| std::fs::remove_file(FILE).unwrap());

		let mut file_stream = FileStream::open(FILE).await.unwrap();
		file_stream.configure(_boundary(
			"boundary-boundary-boundary-boundary-boundary-boundary-boundary-boundary".into(),
		));
	}

	#[tokio::test]
	#[should_panic = "boundary contains non-graphic character"]
	async fn invalid_boundary() {
		const FILE: &str = "panic-5";

		let _ = std::fs::File::create(FILE).unwrap();
		let _deferred = Deferred::call(|| std::fs::remove_file(FILE).unwrap());

		let mut file_stream = FileStream::open(FILE).await.unwrap();
		file_stream.configure(_boundary("boundary\r".into()));
	}
}
