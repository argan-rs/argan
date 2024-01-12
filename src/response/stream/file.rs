use std::{
	fmt::Display,
	fs::File,
	io::{Error as IoError, ErrorKind, Read},
	ops::RangeBounds,
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
	utils::{BoxedError, SCOPE_VALIDITY},
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
	ranges: Vec<RangeValue>,
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
			ranges: Vec::new(),
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
			ranges: Vec::new(),
			some_boundary: None,
			some_content_type_value: None,
			some_file_name: None,
			option_flags: FileStreamOptions::EMPTY,
		})
	}

	pub fn open_ranges<P: AsRef<Path>>(
		path: P,
		range_header_value: &str,
		allow_descending: bool,
	) -> Result<Self, FileStreamError> {
		if range_header_value.is_empty() {
			return Err(FileStreamError::InvalidValue);
		}

		let file = File::open(path)?;

		let metadata = file.metadata()?;
		let file_size = metadata.len();

		let ranges = parse_range_header_value(range_header_value, file_size, allow_descending)?;

		Ok(Self {
			file,
			file_size: file_size.to_string(),
			current_range_index: 0,
			current_range_remaining_size: ranges[0].size(),
			ranges,
			some_content_type_value: None,
			some_boundary: None,
			some_file_name: None,
			option_flags: FileStreamOptions::RANGE_SUPPORT,
		})
	}

	pub fn from_file_ranges(
		file: File,
		range_header_value: &str,
		allow_descending: bool,
	) -> Result<Self, FileStreamError> {
		if range_header_value.is_empty() {
			return Err(FileStreamError::InvalidValue);
		}

		let metadata = file.metadata()?;
		let file_size = metadata.len();

		let ranges = parse_range_header_value(range_header_value, file_size, allow_descending)?;

		Ok(Self {
			file,
			file_size: file_size.to_string(),
			current_range_index: 0,
			current_range_remaining_size: ranges[0].size(),
			ranges,
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
			if !self.ranges.is_empty() {
				self.some_boundary = Some(
					generate_boundary(48)
						.expect("should be valid when the length is no longer than 70 characters"),
				);

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
	if !file_stream.ranges.is_empty() {
		*response.status_mut() = StatusCode::PARTIAL_CONTENT;

		let mut content_range_value = String::new();
		content_range_value.push_str("bytes ");
		content_range_value.push_str(&file_stream.ranges[0].start_str());
		content_range_value.push('-');
		content_range_value.push_str(&file_stream.ranges[0].end_str());
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
	if !file_stream.ranges.is_empty() {
		for range in file_stream.ranges.iter() {
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
			if !self.ranges.is_empty() {
				self.some_boundary = Some(
					generate_boundary(48)
						.expect("should be valid when the length is no longer than 70 characters"),
				);

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
		let next_range_index = file_stream.current_range_index + 1;
		if let Some(next_range_size) = file_stream
			.ranges
			.get(next_range_index)
			.map(|next_range| next_range.size())
		{
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

		if file_stream.ranges.is_empty()
			|| file_stream.current_range_index == file_stream.ranges.len() - 1
		{
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
		let some_range = if !file_stream.ranges.is_empty() {
			Some(&file_stream.ranges[file_stream.current_range_index])
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

fn parse_range_header_value(
	value: &str,
	file_size: u64,
	allow_descending: bool,
) -> Result<Vec<RangeValue>, FileStreamError> {
	let Some(ranges_str) = value.strip_prefix("bytes=") else {
		return Err(FileStreamError::InvalidValue);
	};

	dbg!(ranges_str);

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

					Err(FileStreamError::UnSatisfiableRange)
				})
				.ok()
		})
		.collect::<Vec<RawRangeValue>>();

	if raw_ranges.is_empty() {
		return Err(FileStreamError::UnSatisfiableRange);
	}

	let mut raw_ranges = dbg!(raw_ranges);

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
		return Err(FileStreamError::UnSatisfiableRange);
	}

	let (mut valid_ranges, some_biggest_suffix_range) =
		get_valid_rangges(raw_ranges, ascending_range, file_size)?;

	let mut valid_ranges = dbg!(valid_ranges);
	let some_biggest_suffix_range = dbg!(some_biggest_suffix_range);

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
						return Err(FileStreamError::UnSatisfiableRange);
					}
				} else if !ascending_range {
					return Err(FileStreamError::UnSatisfiableRange);
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
							return Err(FileStreamError::UnSatisfiableRange);
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
							return Err(FileStreamError::UnSatisfiableRange);
						}

						overlap_count += 1;
						merge = true;
					} else if previous_start - current_end.0 < 128 {
						merge = true;
					}
				}
			}

			if merge {
				valid_ranges.last_mut().map(|range| {
					if !ascending_range {
						range.start = current_start;
					}

					if range.end.0 < current_end.0 {
						range.end = current_end;
					}
				});
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
		dbg!(suffix_range_start);
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
				.find_map(|indexed_range| check_position(indexed_range))
			{
				dbg!((position, bigger_than_end, end));
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
		} else {
			if let Some((mut position, bigger_than_end, end)) = valid_ranges
				.iter()
				.enumerate()
				.find_map(|indexed_range| check_position(indexed_range))
			{
				dbg!((position, bigger_than_end, end));
				if !bigger_than_end || suffix_range_start - end < 128 {
					valid_ranges[position].end = (file_end, file_end.to_string().into());
					valid_ranges.rotate_left(position);
					valid_ranges.truncate(valid_ranges.len() - position);
				} else {
					if position > 0 {
						position -= 1;
						valid_ranges[position] = RangeValue::new(suffix_range_start, file_end);
						valid_ranges.rotate_left(position);
						valid_ranges.truncate(valid_ranges.len() - position);
					} else {
						valid_ranges.insert(position, RangeValue::new(suffix_range_start, file_end));
					}
				}
			} else {
				valid_ranges.clear();
				valid_ranges.push(RangeValue::new(suffix_range_start, file_end));
			}
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
			return Err(FileStreamError::InvalidValue);
		};

		let some_start_u64 = if !start.is_empty() {
			if let Ok(start_u64) = start.parse::<u64>() {
				Some(start_u64)
			} else {
				return Err(FileStreamError::InvalidValue);
			}
		} else {
			None
		};

		let some_end_u64 = if !end.is_empty() {
			if let Ok(end_64) = end.parse::<u64>() {
				Some(end_64)
			} else {
				return Err(FileStreamError::InvalidValue);
			}
		} else {
			None
		};

		if some_end_u64
			.is_some_and(|end_u64| some_start_u64.is_some_and(|start_u64| end_u64 < start_u64))
		{
			return Err(FileStreamError::UnSatisfiableRange);
		}

		Ok(RawRangeValue {
			some_start: if let Some(start_u64) = some_start_u64 {
				Some((start_u64, start.into()))
			} else {
				None
			},
			some_end: if let Some(end_u64) = some_end_u64 {
				Some((end_u64, end.into()))
			} else {
				None
			},
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
			return Err(FileStreamError::InvalidValue);
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

// ----------

// ???
#[derive(Debug)]
pub enum FileStreamError {
	InternalServerError,
	InvalidValue,
	UnSatisfiableRange,
	IoError(IoError),
}

impl From<IoError> for FileStreamError {
	fn from(error: IoError) -> Self {
		Self::IoError(error)
	}
}

impl Display for FileStreamError {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "{:?}", self)
	}
}

impl std::error::Error for FileStreamError {}

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

#[cfg(test)]
mod test {
	use super::*;

	#[test]
	fn parse_ranges() {
		const FILE_SIZE: u64 = 10000;
		const END: u64 = FILE_SIZE - 1;

		macro_rules! get_start {
			() => {
				(0, 0.to_string().into())
			};
		}

		macro_rules! get_end {
			() => {
				(END, END.to_string().into())
			};
		}

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
}
