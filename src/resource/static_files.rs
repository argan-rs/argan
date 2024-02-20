use std::{
	fs::Metadata,
	io::Error as IoError,
	os::unix::fs::MetadataExt,
	path::{Path, PathBuf},
	str::FromStr,
	sync::Arc,
};

use http::{
	header::{
		ACCEPT_ENCODING, IF_MATCH, IF_MODIFIED_SINCE, IF_NONE_MATCH, IF_RANGE, IF_UNMODIFIED_SINCE,
		RANGE,
	},
	HeaderMap, HeaderValue, Method, StatusCode,
};
use httpdate::HttpDate;

use crate::{
	common::{patterns_to_route, strip_double_quotes, BoxedError, Uncloneable, SCOPE_VALIDITY},
	handler::{get, request_handlers::handle_mistargeted_request},
	header::{split_header_value, SplitHeaderValueError},
	request::{content_type, Request},
	response::{
		stream::{ContentCoding, FileStream, FileStreamError},
		IntoResponse, Response,
	},
	routing::RoutingState,
};

use super::Resource;

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

const UNENCODED: &'static str = "unencoded";
const ENCODED: &'static str = "encoded";

// --------------------------------------------------

pub struct StaticFiles {
	some_resource: Option<Resource>,
	some_files_dir: Option<Box<Path>>,
	some_tagger: Option<Arc<dyn Tagger>>,
	min_size_to_encode: u64,
	max_size_to_encode: u64,
	level_to_encode: u32,
	flags: Flags,
}

impl StaticFiles {
	pub fn new(resource_path_patterns: impl AsRef<str>, files_dir: impl AsRef<Path>) -> Self {
		let resource = Resource::new(resource_path_patterns.as_ref());

		let files_dir = files_dir.as_ref();

		match files_dir.metadata() {
			Ok(metadata) => {
				if !metadata.is_dir() {
					panic!("{:?} is not a directory", files_dir);
				}
			}
			Err(error) => panic!("{}", error),
		}

		Self {
			some_resource: Some(resource),
			some_files_dir: Some(files_dir.into()),
			some_tagger: None,
			min_size_to_encode: 1024,
			max_size_to_encode: 8 * 1024 * 1024,
			level_to_encode: 6,
			flags: Flags::GET,
		}
	}

	// pub fn with_tagger(mut self, tagger: Arc<dyn Tagger>) -> Self {
	// 	self.some_tagger = Some(tagger);
	//
	// 	self
	// }

	// pub fn with_methods(mut self, allowed_methods: &[Method]) -> Self {
	// 	for method in allowed_methods {
	// 		match method {
	// 			&Method::GET => self.flags.add(Flags::GET),
	// 			&Method::POST => self.flags.add(Flags::POST),
	// 			&Method::DELETE => self.flags.add(Flags::DELETE),
	// 			_ => {}
	// 		}
	// 	}
	//
	// 	self
	// }

	pub fn as_attachments(mut self) -> Self {
		self.flags.add(Flags::ATTACHMENTS);

		self
	}

	pub fn with_dynamic_encoding(mut self) -> Self {
		self.flags.add(Flags::DYNAMIC_ENCODING);

		self
	}

	pub fn with_level_to_encode(mut self, level: u32) -> Self {
		self.level_to_encode = level;

		self
	}

	pub fn with_min_size_to_encode(mut self, min_size_to_encode: u64) -> Self {
		self.min_size_to_encode = min_size_to_encode;

		self
	}

	pub fn with_max_size_to_encode(mut self, max_size_to_encode: u64) -> Self {
		self.max_size_to_encode = max_size_to_encode;

		self
	}

	pub fn into_resource(mut self) -> Resource {
		let mut resource = self
			.some_resource
			.take()
			.expect("resource should be created in the constructor");

		let files_dir = self
			.some_files_dir
			.take()
			.expect("files' dir should be added in the constructor");

		let some_hash_storage = self.some_tagger.take();
		let attachments = self.flags.has(Flags::ATTACHMENTS);
		let dynamic_encoding_props = DynamicEncodingProps {
			enabled: self.flags.has(Flags::DYNAMIC_ENCODING),
			min_file_size: self.min_size_to_encode,
			max_file_size: self.max_size_to_encode,
			level: self.level_to_encode,
		};

		let get_handler = move |request: Request| {
			let files_dir = files_dir.clone();
			let some_hash_storage = some_hash_storage.clone();
			let dynamic_encoding_props = dynamic_encoding_props.clone();

			get_handler(
				request,
				files_dir,
				some_hash_storage,
				attachments,
				dynamic_encoding_props,
			)
		};

		if self.flags.has(Flags::GET) {
			resource.set_handler(get(get_handler));
		}

		resource
	}
}

// -------------------------

/* pub */
trait Tagger: Send + Sync {
	fn get(&self, path: &Path) -> Result<Arc<str>, BoxedError>;
}

// -------------------------

bit_flags! {
	#[derive(Default, Clone)]
	Flags: u8 {
		ATTACHMENTS = 0b0_0001;
		DYNAMIC_ENCODING = 0b0_0010;
		GET = 0b0_0100;
		POST = 0b0_1000;
		DELETE = 0b1_0000;
	}
}

// -------------------------

#[derive(Debug, Clone)]
struct DynamicEncodingProps {
	enabled: bool,
	min_file_size: u64,
	max_file_size: u64,
	level: u32,
}

// --------------------------------------------------

async fn get_handler(
	mut request: Request,
	files_dir: Box<Path>,
	some_hash_storage: Option<Arc<dyn Tagger>>,
	attachments: bool,
	dynamic_encoding_props: DynamicEncodingProps,
) -> Result<Response, StaticFileError> {
	// TODO: We don't need routing state here. Write an extractor to get reamining path segments.
	let routing_state = request
		.extensions_mut()
		.remove::<Uncloneable<RoutingState>>()
		.expect("Uncloneable<RoutingState> should be inserted before routing starts")
		.into_inner()
		.expect("RoutingState should always exist in Uncloneable");

	let request_path = request.uri().path();

	let Some(remaining_segments) = routing_state
		.path_traversal
		.remaining_segments(request_path)
	else {
		return Err(StaticFileError::FileNotFound);
	};

	// TODO: Canonicalize must be tested. We may need to implement it ourselves.
	let relative_path_to_file = AsRef::<Path>::as_ref(remaining_segments).canonicalize()?;

	let (coding, path_buf, should_encode) = evaluate_optimal_coding(
		request.headers(),
		files_dir.as_ref(),
		remaining_segments,
		dynamic_encoding_props.enabled,
		dynamic_encoding_props.min_file_size,
		dynamic_encoding_props.max_file_size,
	)?;

	let path_metadata = match path_buf.metadata() {
		Ok(metadata) => metadata,
		Err(error) => return Err(StaticFileError::IoError(error)),
	};

	let some_file_name = if path_buf.ends_with("gz") {
		path_buf.file_stem()
	} else {
		path_buf.file_name()
	};

	let content_type_value = if let Some(file_name) = some_file_name {
		let mime = mime_guess::from_path(file_name).first_or_else(|| mime::APPLICATION_OCTET_STREAM);

		HeaderValue::from_str(mime.as_ref()).expect("guessed mime type must be a valid header value")
	} else {
		HeaderValue::from_static(mime::APPLICATION_OCTET_STREAM.as_ref())
	};

	let mut file_stream = {
		if dynamic_encoding_props.enabled && should_encode && coding == "gzip" {
			FileStream::open_with_encoding(path_buf, ContentCoding::Gzip(dynamic_encoding_props.level))
				.map_err(Into::<StaticFileError>::into)?
		} else {
			match evaluate_preconditions(
				request.headers(),
				request.method(),
				some_hash_storage,
				&path_buf,
				&path_metadata,
			) {
				PreconditionsResult::None => {
					FileStream::open(path_buf).map_err(Into::<StaticFileError>::into)?
				}
				PreconditionsResult::Ranges(ranges) => {
					FileStream::open_ranges(path_buf, ranges, true).map_err(Into::<StaticFileError>::into)?
				}
				PreconditionsResult::NotModified => return Ok(StatusCode::NOT_MODIFIED.into_response()),
				PreconditionsResult::Failed => return Ok(StatusCode::PRECONDITION_FAILED.into_response()),
				PreconditionsResult::InvalidDate => return Err(StaticFileError::InvalidHttpDate),
				PreconditionsResult::IoError(error) => return Err(StaticFileError::IoError(error)),
			}
		}
	};

	if attachments {
		file_stream.as_attachment();
	}

	file_stream.set_content_type(content_type_value);

	if coding == "gzip" {
		let _ = file_stream
			.set_content_encoding(HeaderValue::from_static("gzip"))
			.map_err(Into::<StaticFileError>::into)?;
	}

	Ok(file_stream.into_response())
}

// ----------

fn evaluate_optimal_coding<'h, P1: AsRef<Path>, P2: AsRef<Path>>(
	request_headers: &'h HeaderMap,
	files_dir: P1,
	relative_path_to_file: P2,
	dynamic_compression: bool,
	min_size_to_compress: u64,
	max_size_to_compress: u64,
) -> Result<(&'h str, PathBuf, bool), StaticFileError> {
	let files_dir = files_dir.as_ref();
	let relative_path_to_file = relative_path_to_file.as_ref();

	if let Some(header_value) = request_headers.get(ACCEPT_ENCODING) {
		let elements = split_header_value(header_value)?;

		let some_path_buf = if let Some(position) = elements.iter().position(
			|(value, _)| /* value.eq_ignore_ascii_case("br") || */ value.eq_ignore_ascii_case("gzip"),
		) {
			let /* encoding */ preferred_encoding = elements[position];
			// let other_encoding_name = if encoding.0 == "br" { "gzip" } else { "br" };
			//
			// let other_encoding = (other_encoding_name, 0.0);
			// let other_encoding = elements[position + 1..]
			// 	.iter()
			// 	.find(|(value, _)| value.eq_ignore_ascii_case(other_encoding_name))
			// 	.or(Some(&other_encoding))
			// 	.expect(SCOPE_VALIDITY);
			//
			// let (preferred_encoding, other_encoding) = if encoding.1 > other_encoding.1 {
			// 	(&encoding, other_encoding)
			// } else {
			// 	(other_encoding, &encoding)
			// };

			let some_path_buf = if preferred_encoding.1 > 0.0 {
				let mut path_buf = files_dir
					.join(ENCODED)
					.join(preferred_encoding.0)
					.join(relative_path_to_file);

				if path_buf.is_file() {
					return Ok((preferred_encoding.0, path_buf, false));
				}

				// if other_encoding.1 > 0.0 {
				// 	path_buf.clear();
				//
				// 	path_buf.push(files_dir);
				// 	path_buf.push(COMPRESSED);
				// 	path_buf.push(other_encoding.0);
				// 	path_buf.push(relative_path_to_file);
				//
				// 	if path_buf.is_file() {
				// 		return Ok((other_encoding.0, path_buf, false));
				// 	}
				// }

				Some(path_buf)
			} else {
				None
			};

			let path_buf = if let Some(mut path_buf) = some_path_buf {
				path_buf.clear();

				path_buf.push(files_dir);
				path_buf.push(UNENCODED);
				path_buf.push(relative_path_to_file);

				path_buf
			} else {
				files_dir.join(UNENCODED).join(relative_path_to_file)
			};

			if !path_buf.is_file() {
				return Err(StaticFileError::FileNotFound);
			}

			match path_buf.metadata() {
				Ok(metadata) => {
					if dynamic_compression {
						let file_size = metadata.size();
						if file_size >= min_size_to_compress && file_size <= max_size_to_compress {
							if preferred_encoding.1 > 0.0 {
								return Ok((preferred_encoding.0, path_buf, true));
							}

							// if other_encoding.1 > 0.0 {
							// 	return Ok((other_encoding.0, path_buf, true));
							// }
						}
					}

					Some(path_buf)
				}
				Err(io_error) => return Err(io_error.into()),
			}
		} else {
			None
		};

		if elements.iter().any(|(value, weight)| {
			(value.eq_ignore_ascii_case("identity") && *weight == 0.0)
				|| (value.eq_ignore_ascii_case("*") && *weight == 0.0)
		}) {
			if some_path_buf.is_some() {
				// Identity is forbidden. Elements cointain gzip, but we don't have
				// the compressed file, and we can't dynamically compress.
				return Err(StaticFileError::AcceptEncoding("identity"));
			}

			// Elements don't have gzip, and identity is forbidden.

			let path_buf = files_dir.join(UNENCODED).join(relative_path_to_file);
			if !path_buf.is_file() {
				return Err(StaticFileError::FileNotFound);
			}

			/* return Err(StaticFileError::AcceptEncoding("br, gzip, identity")); */
			return Err(StaticFileError::AcceptEncoding("gzip, identity"));
		}

		if let Some(path_buf) = some_path_buf {
			return Ok(("", path_buf, false));
		}
	}

	let path_buf = files_dir.join(UNENCODED).join(relative_path_to_file);
	if !path_buf.is_file() {
		return Err(StaticFileError::FileNotFound);
	}

	Ok(("", path_buf, false))
}

fn evaluate_preconditions<'r>(
	request_headers: &'r HeaderMap,
	request_method: &Method,
	some_hash_storage: Option<Arc<dyn Tagger>>,
	path: &Path,
	path_metadata: &Metadata,
) -> PreconditionsResult<'r> {
	let mut check_the_next_step = true;
	let mut some_file_hash = None;

	if let Some(hash_storage) = some_hash_storage.as_ref() {
		if let Some(hashes_to_match) = request_headers.get(IF_MATCH).map(|value| value.as_bytes()) {
			// step 1
			if hashes_to_match != b"*" {
				match hash_storage.get(&path) {
					Ok(file_hash) => {
						if !hashes_to_match
							.split(|ch| *ch == b',')
							.any(|hash_to_match| file_hash.as_bytes() == strip_double_quotes(hash_to_match))
						{
							return PreconditionsResult::Failed;
						}

						some_file_hash = Some(file_hash);
					}
					Err(_) => return PreconditionsResult::Failed,
				}
			}

			// When IF-MATCH exists, we ignore the step 2 IF-UNMODIFIED-SINCE.
			check_the_next_step = false;
		}
	}

	if check_the_next_step {
		if let Some(time_to_match) = request_headers
			.get(IF_UNMODIFIED_SINCE)
			.and_then(|value| value.to_str().ok())
		{
			// step 2
			let modified_time = match path_metadata.modified() {
				Ok(modified_time) => modified_time,
				Err(error) => return PreconditionsResult::IoError(error),
			};

			let Ok(http_date_to_match) = HttpDate::from_str(time_to_match) else {
				return PreconditionsResult::InvalidDate;
			};

			if HttpDate::from(modified_time) != http_date_to_match {
				return PreconditionsResult::Failed;
			}
		}
	}

	check_the_next_step = true;

	if let Some(hash_storage) = some_hash_storage.as_ref() {
		if let Some(hashes_to_match) = request_headers
			.get(IF_NONE_MATCH)
			.map(|value| value.as_bytes())
		{
			// step 3
			let precondition_failed = if hashes_to_match == b"*" {
				true
			} else {
				if some_file_hash.is_none() {
					some_file_hash = hash_storage.get(&path).ok();
				};

				if let Some(file_hash) = some_file_hash.as_ref() {
					hashes_to_match
						.split(|ch| *ch == b',')
						.any(|hash_to_match| {
							let hash_to_match = if hash_to_match.starts_with(b"W/") {
								&hash_to_match[2..]
							} else {
								hash_to_match
							};

							file_hash.as_bytes() == strip_double_quotes(hash_to_match)
						})
				} else {
					false
				}
			};

			if precondition_failed {
				if request_method == Method::GET || request_method == Method::HEAD {
					return PreconditionsResult::NotModified;
				}

				return PreconditionsResult::Failed;
			}

			check_the_next_step = false;
		}
	}

	if check_the_next_step {
		if request_method == Method::GET || request_method == Method::HEAD {
			// step 4
			if let Some(time_to_match) = request_headers
				.get(IF_MODIFIED_SINCE)
				.and_then(|value| value.to_str().ok())
			{
				let modified_time = match path_metadata.modified() {
					Ok(modified_time) => modified_time,
					Err(error) => return PreconditionsResult::IoError(error),
				};

				let Ok(http_date_to_match) = HttpDate::from_str(time_to_match) else {
					return PreconditionsResult::InvalidDate;
				};

				if HttpDate::from(modified_time) == http_date_to_match {
					return PreconditionsResult::NotModified;
				}
			}
		}
	}

	if request_method == Method::GET || request_method == Method::HEAD {
		let some_ranges_str = request_headers
			.get(RANGE)
			.and_then(|value| value.to_str().ok());

		if some_ranges_str.is_some() {
			// setp 5
			if let Some(range_precondition) = request_headers
				.get(IF_RANGE)
				.and_then(|value| value.to_str().ok())
			{
				if range_precondition.starts_with("W/") {
					return PreconditionsResult::None;
				}

				if range_precondition.starts_with('"') {
					let Some(hash_storage) = some_hash_storage else {
						return PreconditionsResult::None;
					};

					let hash_to_match = strip_double_quotes(range_precondition.as_bytes());

					let file_hash = if let Some(file_hash) = some_file_hash {
						file_hash
					} else {
						match hash_storage.get(&path) {
							Ok(file_hash) => file_hash,
							Err(_) => return PreconditionsResult::None,
						}
					};

					if file_hash.as_bytes() == hash_to_match {
						return PreconditionsResult::Ranges(some_ranges_str.expect(SCOPE_VALIDITY));
					}
				} else {
					let modified_time = match path_metadata.modified() {
						Ok(modified_time) => modified_time,
						Err(error) => return PreconditionsResult::IoError(error),
					};

					let Ok(http_date_to_match) = HttpDate::from_str(range_precondition) else {
						return PreconditionsResult::InvalidDate;
					};

					if HttpDate::from(modified_time) == http_date_to_match {
						return PreconditionsResult::Ranges(some_ranges_str.expect(SCOPE_VALIDITY));
					}
				}
			} else {
				return PreconditionsResult::Ranges(some_ranges_str.expect(SCOPE_VALIDITY));
			}
		}
	}

	PreconditionsResult::None
}

enum PreconditionsResult<'r> {
	None,
	Ranges(&'r str),
	NotModified,
	Failed,
	IoError(IoError),
	InvalidDate,
}

// --------------------------------------------------

#[non_exhaustive]
#[derive(Debug, crate::ImplError)]
pub enum StaticFileError {
	#[error(transparent)]
	InvalidAcceptEncoding(#[from] SplitHeaderValueError),
	#[error("invalid HTTP date")]
	InvalidHttpDate,
	#[error("file not found")]
	FileNotFound,
	#[error("Accept-Encoding must be {0}")]
	AcceptEncoding(&'static str),
	#[error(transparent)]
	IoError(#[from] IoError),
	#[error(transparent)]
	FileStreamFailure(#[from] FileStreamError),
}

impl IntoResponse for StaticFileError {
	fn into_response(self) -> Response {
		let mut response = Response::default();

		match self {
			Self::InvalidAcceptEncoding(_) | Self::InvalidHttpDate => {
				*response.status_mut() = StatusCode::BAD_REQUEST
			}
			Self::FileNotFound => *response.status_mut() = StatusCode::NOT_FOUND,
			Self::AcceptEncoding(codings) => {
				response
					.headers_mut()
					.insert(ACCEPT_ENCODING, HeaderValue::from_static(codings));
			}
			Self::IoError(_) => *response.status_mut() = StatusCode::INTERNAL_SERVER_ERROR,
			Self::FileStreamFailure(error) => return error.into_response(),
		}

		response
	}
}

// --------------------------------------------------------------------------------

#[cfg(test)]
mod test {}
