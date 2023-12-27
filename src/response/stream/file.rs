use std::{
	fs::File,
	io::{Error as IoError, ErrorKind, Read},
	path::Path,
	pin::Pin,
	str::FromStr,
	sync::Arc,
	task::{Context, Poll},
	time::{SystemTime, UNIX_EPOCH},
	usize,
};

use bytes::{BufMut, Bytes, BytesMut};
use http::{
	header::{ACCEPT_RANGES, CONTENT_DISPOSITION, CONTENT_LENGTH, CONTENT_RANGE, CONTENT_TYPE},
	HeaderValue, StatusCode,
};
use http_body_util::BodyExt;
use hyper::body::{Body, Frame};
use percent_encoding::{percent_encode, NON_ALPHANUMERIC};
use rand::{rngs::SmallRng, Rng, SeedableRng};

use crate::{
	response::{IntoResponse, Response},
	utils::BoxedError,
};

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

const BUFFER_SIZE: usize = 8 * 1024;

// -------------------------

// TODO: Add tests.

pub struct FileStream {
	file: File,
	file_size: String,
	current_range_index: usize,
	current_range_remaining_size: u64,
	some_ranges: Option<Vec<RangeValue>>,
	some_boundary: Option<Arc<str>>,
	some_content_type_value: Option<HeaderValue>,
	some_file_name: Option<String>,
	option_flags: FileStreamOptions,
}

impl FileStream {
	pub fn open<P: AsRef<Path>>(path: P) -> Result<Self, FileStreamError> {
		let file = File::open(path)?;

		let metadata = file.metadata()?;
		let file_size = metadata.len();

		Ok(Self {
			file,
			file_size: file_size.to_string(),
			current_range_index: 0,
			current_range_remaining_size: file_size,
			some_ranges: None,
			some_boundary: None,
			some_content_type_value: None,
			some_file_name: None,
			option_flags: FileStreamOptions::EMPTY,
		})
	}

	pub fn from_file(file: File) -> Result<Self, FileStreamError> {
		let metadata = file.metadata()?;
		let file_size = metadata.len();

		Ok(Self {
			file,
			file_size: file_size.to_string(),
			current_range_index: 0,
			current_range_remaining_size: file_size,
			some_ranges: None,
			some_boundary: None,
			some_content_type_value: None,
			some_file_name: None,
			option_flags: FileStreamOptions::EMPTY,
		})
	}

	pub fn open_ranges<P: AsRef<Path>>(
		path: P,
		ranges: Vec<RangeValue>,
	) -> Result<Self, FileStreamError> {
		if ranges.is_empty() {
			return Err(FileStreamError::NoRange);
		}

		let file = File::open(path)?;

		let metadata = file.metadata()?;
		let file_size = metadata.len();

		if ranges.iter().any(|range| range.end() >= file_size) {
			return Err(FileStreamError::RangeNotSatisfiable);
		}

		Ok(Self {
			file,
			file_size: file_size.to_string(),
			current_range_index: 0,
			current_range_remaining_size: ranges[0].size(),
			some_ranges: Some(ranges),
			some_content_type_value: None,
			some_boundary: None,
			some_file_name: None,
			option_flags: FileStreamOptions::RANGE_SUPPORT,
		})
	}

	pub fn from_file_ranges(file: File, ranges: Vec<RangeValue>) -> Result<Self, FileStreamError> {
		if ranges.is_empty() {
			return Err(FileStreamError::NoRange);
		}

		let metadata = file.metadata()?;
		let file_size = metadata.len();

		if ranges.iter().any(|range| range.end() >= file_size) {
			return Err(FileStreamError::RangeNotSatisfiable);
		}

		Ok(Self {
			file,
			file_size: file_size.to_string(),
			current_range_index: 0,
			current_range_remaining_size: ranges[0].size(),
			some_ranges: Some(ranges),
			some_content_type_value: None,
			some_boundary: None,
			some_file_name: None,
			option_flags: FileStreamOptions::RANGE_SUPPORT,
		})
	}

	#[inline(always)]
	pub fn set_boundary(&mut self, boundary: Arc<str>) -> Result<(), FileStreamError> {
		if boundary.chars().any(|ch| !ch.is_ascii_graphic()) {
			return Err(FileStreamError::InvalidValue);
		}

		self.some_boundary = Some(boundary);

		Ok(())
	}

	#[inline(always)]
	pub fn set_content_type(&mut self, value: HeaderValue) {
		self.some_content_type_value = Some(value);
	}

	#[inline]
	pub fn set_file_name(&mut self, file_name: &str) {
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
			file_name_string.push_str(file_name);
			file_name_string.push('"');
		}

		self.some_file_name = Some(file_name_string);
	}

	#[inline(always)]
	pub fn set_options(&mut self, options: FileStreamOptions) {
		self.option_flags.add(options);
	}
}

impl IntoResponse for FileStream {
	fn into_response(mut self) -> crate::response::Response {
		if self.some_boundary.is_some() {
			multipart_ranges_response(self)
		} else {
			if self
				.some_ranges
				.as_ref()
				.is_some_and(|ranges| ranges.len() > 1)
			{
				self.some_boundary =
					Some(generate_boundary(48).expect("the boundary must not be longer than 70 characters"));

				multipart_ranges_response(self)
			} else {
				single_range_response(self)
			}
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
	if let Some(content_type_value) = file_stream.some_content_type_value.take() {
		response
			.headers_mut()
			.insert(CONTENT_TYPE, content_type_value);
	}

	// Non-multipart responses may have a single range. It's expected to be checked before
	// the function is called.
	if let Some(ranges) = file_stream.some_ranges.as_ref() {
		*response.status_mut() = StatusCode::PARTIAL_CONTENT;

		let mut content_range_value = String::new();
		content_range_value.push_str("bytes ");
		content_range_value.push_str(&ranges[0].start_str());
		content_range_value.push('-');
		content_range_value.push_str(&ranges[0].end_str());
		content_range_value.push('/');
		content_range_value.push_str(&file_stream.file_size);

		insert_header!(response, CONTENT_RANGE, &content_range_value);
	} else {
		let some_content_disposition_value =
			if file_stream.option_flags.has(FileStreamOptions::ATTACHMENT) {
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

		if file_stream
			.option_flags
			.has(FileStreamOptions::RANGE_SUPPORT)
		{
			response
				.headers_mut()
				.insert(ACCEPT_RANGES, HeaderValue::from_static("bytes"));
		}
	}

	// current_range_remaining_size is either a file size or a single range size.
	insert_header!(
		response,
		CONTENT_LENGTH,
		&file_stream.current_range_remaining_size.to_string()
	);

	*response.body_mut() = file_stream.boxed();

	response
}

fn multipart_ranges_response(file_stream: FileStream) -> Response {
	let boundary = file_stream
		.some_boundary
		.as_ref()
		.expect("streaming multipart ranges shouldn't start without a boundary")
		.clone();

	let mut response = Response::default();

	{
		let value_part = "multipart/byteranges; boundary=";
		let mut header_value = String::with_capacity(value_part.len() + boundary.len());
		header_value.push_str(value_part);
		header_value.push_str(&boundary);

		insert_header!(response, CONTENT_TYPE, &header_value);
	}

	let mut body_size = 0u64;
	if let Some(ranges) = &file_stream.some_ranges {
		for range in ranges {
			body_size += part_header_size(
				boundary.len(),
				file_stream.some_content_type_value.as_ref(),
				Some(range),
				&file_stream.file_size,
			) as u64;

			body_size += range.size();
		}
	} else {
		body_size += part_header_size(
			boundary.len(),
			file_stream.some_content_type_value.as_ref(),
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

	*response.body_mut() = file_stream.boxed();

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

impl Body for FileStream {
	type Data = Bytes;
	type Error = BoxedError;

	fn poll_frame(
		mut self: Pin<&mut Self>,
		cx: &mut Context<'_>,
	) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
		if self.some_boundary.is_some() {
			stream_multipart_ranges(self, cx)
		} else {
			if self
				.some_ranges
				.as_ref()
				.is_some_and(|ranges| ranges.len() > 1)
			{
				self.some_boundary =
					Some(generate_boundary(48).expect("the boundary must not be longer than 70 characters"));

				stream_multipart_ranges(self, cx)
			} else {
				stream_single_range(self, cx)
			}
		}
	}
}

fn stream_single_range(
	mut file_stream: Pin<&mut FileStream>,
	cx: &mut Context<'_>,
) -> Poll<Option<Result<Frame<Bytes>, BoxedError>>> {
	if file_stream.current_range_remaining_size == 0 {
		return Poll::Ready(None);
	}

	let mut buffer = if file_stream.current_range_remaining_size < BUFFER_SIZE as u64 {
		BytesMut::zeroed(file_stream.current_range_remaining_size as usize)
	} else {
		BytesMut::zeroed(BUFFER_SIZE)
	};

	for _ in 0..3 {
		match file_stream.file.read(&mut buffer) {
			Ok(size) => {
				file_stream.current_range_remaining_size -= size as u64;
				buffer.truncate(size);

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
		if let Some((true, next_range_index, next_range_size)) =
			file_stream.some_ranges.as_ref().map(|ranges| {
				let next_range_index = file_stream.current_range_index + 1;
				if let Some(next_range) = ranges.get(next_range_index) {
					(true, next_range_index, next_range.size())
				} else {
					(false, 0, 0)
				}
			}) {
			file_stream.current_range_index = next_range_index;
			file_stream.current_range_remaining_size = next_range_size;

			true
		} else {
			return Poll::Ready(None);
		}
	} else {
		false
	};

	let boundary = file_stream
		.some_boundary
		.as_ref()
		.expect("streaming multipart ranges shouldn't start without a boundary")
		.clone();

	let (capacity, last_one) = if file_stream.current_range_remaining_size <= BUFFER_SIZE as u64 {
		let capacity = file_stream.current_range_remaining_size as usize;

		if file_stream.some_ranges.as_ref().map_or(true, |ranges| {
			file_stream.current_range_index == ranges.len() - 1
		}) {
			//          |     |       |     |
			//          \r\n--boundary--\r\n
			(capacity + 4 + boundary.len() + 4, true)
		} else {
			(capacity, false)
		}
	} else {
		(BUFFER_SIZE, false)
	};

	let (mut buffer, start_index) = if new_part {
		let some_range = if let Some(ranges) = file_stream.some_ranges.as_ref() {
			Some(&ranges[file_stream.current_range_index])
		} else {
			None
		};

		let part_header_size = part_header_size(
			boundary.len(),
			file_stream.some_content_type_value.as_ref(),
			some_range,
			&file_stream.file_size,
		);

		let mut buffer = BytesMut::with_capacity(capacity + part_header_size);
		buffer.put_slice(b"\r\n--");
		buffer.put_slice(boundary.as_bytes());

		if let Some(content_type_value) = file_stream.some_content_type_value.as_ref() {
			buffer.put_slice(b"\r\n");
			buffer.put_slice(CONTENT_TYPE.as_str().as_bytes());
			buffer.put_slice(b": ");
			buffer.put_slice(content_type_value.as_bytes());
		}

		buffer.put_slice(b"\r\n");
		buffer.put_slice(CONTENT_RANGE.as_str().as_bytes());
		buffer.put_slice(b": bytes ");
		buffer.put_slice(some_range.map_or(b"0", |range| range.start_string.as_bytes()));
		buffer.put_u8(b'-');
		buffer.put_slice(
			some_range.map_or(file_stream.file_size.as_bytes(), |range| {
				range.end_string.as_bytes()
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

	for _ in 0..3 {
		match file_stream.file.read(&mut buffer[start_index..]) {
			Ok(size) => {
				file_stream.current_range_remaining_size -= size as u64;
				buffer.resize(start_index + size, 0);

				if last_one && file_stream.current_range_remaining_size == 0 {
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

pub fn generate_boundary(length: u8) -> Result<Arc<str>, FileStreamError> {
	if length == 0 || length > 70 {
		return Err(FileStreamError::InvalidValue);
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

bit_flags! {
	pub FileStreamOptions: u8 {
		EMPTY = 0b00;
		pub ATTACHMENT = 0b01;
		pub RANGE_SUPPORT = 0b10;
	}
}

// ----------

pub struct RangeValue {
	start: u64,
	start_string: String,
	end: u64,
	end_string: String,
}

impl FromStr for RangeValue {
	type Err = FileStreamError;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		let Some(mut range_value) = s.split_once('-').map(|(start, end)| RangeValue {
			start: 0,
			start_string: start.to_owned(),
			end: 0,
			end_string: end.to_owned(),
		}) else {
			return Err(FileStreamError::InvalidValue);
		};

		let Ok(start) = range_value.start_string.parse::<u64>() else {
			return Err(FileStreamError::InvalidValue);
		};

		let Ok(end) = range_value.end_string.parse::<u64>() else {
			return Err(FileStreamError::InvalidValue);
		};

		if end < start {
			return Err(FileStreamError::RangeNotSatisfiable);
		}

		range_value.start = start;
		range_value.end = end;

		Ok(range_value)
	}
}

impl RangeValue {
	#[inline(always)]
	pub fn start(&self) -> u64 {
		self.start
	}

	#[inline(always)]
	pub fn start_str(&self) -> &str {
		&self.start_string
	}

	#[inline(always)]
	pub fn end(&self) -> u64 {
		self.end
	}

	#[inline(always)]
	pub fn end_str(&self) -> &str {
		&self.end_string
	}

	#[inline(always)]
	pub fn len(&self) -> usize {
		// |    ||  |
		// start-end
		self.start_string.len() + 1 + self.end_string.len()
	}

	#[inline(always)]
	pub fn size(&self) -> u64 {
		self.end - self.start + 1 // end is inclusive
	}
}

// ----------

// ???
#[derive(Debug)]
pub enum FileStreamError {
	NoRange,
	InternalServerError,
	InvalidValue,
	RangeNotSatisfiable,
	IoError(IoError),
}

impl From<IoError> for FileStreamError {
	fn from(error: IoError) -> Self {
		Self::IoError(error)
	}
}

// --------------------------------------------------------------------------------
