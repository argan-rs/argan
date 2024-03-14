use std::{
	ffi::OsStr,
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
	common::{
		normalize_path, patterns_to_route, strip_double_quotes, BoxedError, BoxedFuture, Uncloneable,
		SCOPE_VALIDITY,
	},
	handler::{_get, request_handlers::handle_mistargeted_request, Handler, IntoHandler},
	header::{split_header_value, SplitHeaderValueError},
	request::{FromRequest, RemainingPath, Request},
	response::{
		file_stream::{ContentCoding, FileStream, FileStreamError},
		IntoResponse, IntoResponseResult, Response,
	},
	routing::RoutingState,
};

use super::{config::_as_subtree_handler, Resource};

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
			flags: Flags::PARTIAL_CONTENT | Flags::DYNAMIC_ENCODING | Flags::GET,
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

	pub fn with_encoding_level(mut self, level: u32) -> Self {
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

		resource.configure(_as_subtree_handler());

		let files_dir = self
			.some_files_dir
			.take()
			.expect("files' dir should be added in the constructor");

		// let some_tagger = self.some_tagger.take();
		let attachments = self.flags.has(Flags::ATTACHMENTS);
		let partial_content_support = self.flags.has(Flags::PARTIAL_CONTENT);
		let dynamic_encoding_props = DynamicEncodingProps {
			enabled: self.flags.has(Flags::DYNAMIC_ENCODING),
			min_file_size: self.min_size_to_encode,
			max_file_size: self.max_size_to_encode,
			level: self.level_to_encode,
		};

		let get_handler = move |remaining_path: RemainingPath, request: Request| {
			let handler_props = HandlerProps {
				files_dir: files_dir.clone(),
				some_tagger: None, /* some_tagger.clone() */
				attachments,
				partial_content_support,
				dynamic_encoding_props: dynamic_encoding_props.clone(),
			};

			get_handler(request, remaining_path, handler_props)
		};

		if self.flags.has(Flags::GET) {
			resource.set_handler_for(_get(get_handler));
		}

		resource
	}
}

// -------------------------

/* pub */
trait Tagger: Send + Sync {
	fn get(&self, path: &Path) -> Result<Box<str>, BoxedError>;
}

// -------------------------

bit_flags! {
	#[derive(Default, Clone)]
	Flags: u8 {
		ATTACHMENTS = 0b00_0001;
		PARTIAL_CONTENT = 0b00_0010;
		DYNAMIC_ENCODING = 0b00_0100;
		GET = 0b00_1000;
		POST = 0b01_0000;
		DELETE = 0b10_0000;
	}
}

// -------------------------

#[derive(Clone)]
struct HandlerProps {
	files_dir: Box<Path>,
	some_tagger: Option<Arc<dyn Tagger>>,
	attachments: bool,
	partial_content_support: bool,
	dynamic_encoding_props: DynamicEncodingProps,
}

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
	remaining_path: RemainingPath,
	HandlerProps {
		files_dir,
		some_tagger,
		attachments,
		partial_content_support,
		dynamic_encoding_props,
	}: HandlerProps,
) -> Result<Response, StaticFileError> {
	let RemainingPath::Value(remaining_path) = remaining_path else {
		return Err(StaticFileError::FileNotFound);
	};

	let remaining_path = normalize_path(remaining_path.as_ref());

	let (coding, path_buf, should_encode) = evaluate_optimal_coding(
		request.headers(),
		files_dir.as_ref(),
		&remaining_path,
		dynamic_encoding_props.enabled,
		dynamic_encoding_props.min_file_size,
		dynamic_encoding_props.max_file_size,
	)?;

	let path_metadata = match path_buf.metadata() {
		Ok(metadata) => metadata,
		Err(error) => return Err(StaticFileError::IoError(error)),
	};

	let mime =
		mime_guess::from_path(&remaining_path).first_or_else(|| mime::APPLICATION_OCTET_STREAM);

	let content_type_value =
		HeaderValue::from_str(mime.as_ref()).expect("guessed mime type must be a valid header value");

	let mut file_stream = {
		if dynamic_encoding_props.enabled && should_encode && coding == "gzip" {
			FileStream::open_with_encoding(path_buf, ContentCoding::Gzip(dynamic_encoding_props.level))
				.map_err(Into::<StaticFileError>::into)?
		} else {
			let mut file_stream = match evaluate_preconditions(
				request.headers(),
				request.method(),
				some_tagger,
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
			};

			file_stream.support_partial_content();

			file_stream
		}
	};

	if attachments {
		file_stream.as_attachment();
	}

	if coding == "gzip" {
		file_stream.set_content_encoding(HeaderValue::from_static("gzip"));
	}

	file_stream.set_content_type(content_type_value);

	Ok(file_stream.into_response())
}

// ----------

fn evaluate_optimal_coding<'h, P1: AsRef<Path>, P2: AsRef<str>>(
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
				let relative_path_to_file = match preferred_encoding.0 {
					"gzip" => {
						let mut relative_path_to_file = relative_path_to_file.to_string();
						relative_path_to_file.push_str(".gz");

						relative_path_to_file
					}

					_ => relative_path_to_file.to_string(),
				};

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
mod test {
	use std::fs;

	use bytes::Bytes;
	use http::header::{
		ACCEPT_RANGES, CONTENT_ENCODING, CONTENT_LENGTH, CONTENT_RANGE, CONTENT_TYPE,
	};
	use http_body_util::{BodyExt, Empty};
	use hyper::service::Service;

	use crate::common::Deferred;

	use super::*;

	// --------------------------------------------------------------------------------
	// --------------------------------------------------------------------------------

	const FILE_SIZE: usize = 32 * 1024 * 1024;

	// --------------------------------------------------

	#[tokio::test]
	async fn static_files() {
		// ----------
		// Dirs

		let root_dir: PathBuf = "test/".into();
		let unencoded_dir = root_dir.join(UNENCODED);
		let encoded_dir = root_dir.join(ENCODED).join("gzip");

		let unencoded_small_dir = unencoded_dir.join("small");
		let encoded_small_dir = encoded_dir.join("small");

		fs::create_dir_all(&unencoded_dir).unwrap();
		fs::create_dir_all(&encoded_dir).unwrap();

		fs::create_dir_all(&unencoded_small_dir).unwrap();
		fs::create_dir_all(&encoded_small_dir).unwrap();

		// ----------
		// Contents

		let mut contents_32m = Vec::with_capacity(FILE_SIZE);
		for i in 0..FILE_SIZE {
			contents_32m.push({ i % 7 } as u8);
		}

		let mut contents_16m = Vec::with_capacity(FILE_SIZE / 2);
		for i in 0..FILE_SIZE / 2 {
			contents_16m.push({ i % 7 } as u8);
		}

		let mut contents_8m = Vec::with_capacity(FILE_SIZE / 4);
		for i in 0..FILE_SIZE / 4 {
			contents_8m.push({ i % 7 } as u8);
		}

		let mut contents_4m = Vec::with_capacity(FILE_SIZE / 8);
		for i in 0..FILE_SIZE / 8 {
			contents_4m.push({ i % 7 } as u8);
		}

		let mut contents_2m = Vec::with_capacity(FILE_SIZE / 16);
		for i in 0..FILE_SIZE / 16 {
			contents_2m.push({ i % 7 } as u8);
		}

		let mut contents_1m = Vec::with_capacity(FILE_SIZE / 32);
		for i in 0..FILE_SIZE / 32 {
			contents_1m.push({ i % 7 } as u8);
		}

		// ----------
		// Files

		let _ = fs::write(unencoded_dir.join("text_32m.txt"), &contents_32m);
		let _ = fs::write(unencoded_dir.join("html_32m.html"), &contents_32m);
		let _ = fs::write(unencoded_dir.join("text_16m.txt"), &contents_16m);
		let _ = fs::write(unencoded_dir.join("html_16m.html"), &contents_16m);
		let _ = fs::write(unencoded_small_dir.join("text_8m.txt"), &contents_8m);
		let _ = fs::write(unencoded_small_dir.join("html_8m.html"), &contents_8m);
		let _ = fs::write(unencoded_small_dir.join("text_4m.txt"), &contents_4m);
		let _ = fs::write(unencoded_small_dir.join("html_4m.html"), &contents_4m);
		let _ = fs::write(unencoded_small_dir.join("text_2m.txt"), &contents_2m);
		let _ = fs::write(unencoded_small_dir.join("html_2m.html"), &contents_2m);
		let _ = fs::write(unencoded_small_dir.join("text_1m.txt"), &contents_1m);
		let _ = fs::write(unencoded_small_dir.join("html_1m.html"), &contents_1m);

		let _ = fs::write(encoded_dir.join("text_32m.txt.gz"), &contents_32m);
		let _ = fs::write(encoded_dir.join("html_32m.html.gz"), &contents_32m);
		let _ = fs::write(encoded_small_dir.join("text_8m.txt.gz"), &contents_8m);
		let _ = fs::write(encoded_small_dir.join("html_8m.html.gz"), &contents_8m);

		let _deferred = Deferred::call(|| fs::remove_dir_all(&root_dir).unwrap());

		// --------------------------------------------------

		let mut static_files = StaticFiles::new("/files", &root_dir)
			.into_resource()
			.into_service();

		// -------------------------

		let request = Request::get("/files").body(Empty::default()).unwrap();
		let response = static_files.call(request).await.unwrap();
		assert_eq!(StatusCode::NOT_FOUND, response.status());

		// -------------------------

		let request = Request::get("/files/text_32m.txt")
			.body(Empty::default())
			.unwrap();

		let response = static_files.call(request).await.unwrap();
		assert_eq!(StatusCode::OK, response.status());
		assert_eq!(
			mime::TEXT_PLAIN.as_ref(),
			response
				.headers()
				.get(CONTENT_TYPE)
				.unwrap()
				.to_str()
				.unwrap()
		);

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
			contents_32m.len().to_string(),
			response
				.headers()
				.get(CONTENT_LENGTH)
				.unwrap()
				.to_str()
				.unwrap()
		);

		dbg!(response.headers());

		// ----------

		let request = Request::get("/files/text_32m.txt")
			.header(ACCEPT_ENCODING, "gzip")
			.body(Empty::default())
			.unwrap();

		let response = static_files.call(request).await.unwrap();
		assert_eq!(StatusCode::OK, response.status());
		assert_eq!(
			mime::TEXT_PLAIN.as_ref(),
			response
				.headers()
				.get(CONTENT_TYPE)
				.unwrap()
				.to_str()
				.unwrap()
		);

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
			contents_32m.len().to_string(),
			response
				.headers()
				.get(CONTENT_LENGTH)
				.unwrap()
				.to_str()
				.unwrap()
		);

		// text_32m.txt does have an encoded copy.
		assert_eq!(
			"gzip",
			response
				.headers()
				.get(CONTENT_ENCODING)
				.unwrap()
				.to_str()
				.unwrap()
		);

		dbg!(response.headers());

		// -------------------------

		let request = Request::get("/files/html_32m.html")
			.header(RANGE, "bytes=-1024")
			.body(Empty::default())
			.unwrap();

		let response = static_files.call(request).await.unwrap();
		assert_eq!(StatusCode::PARTIAL_CONTENT, response.status());
		assert_eq!(
			mime::TEXT_HTML.as_ref(),
			response
				.headers()
				.get(CONTENT_TYPE)
				.unwrap()
				.to_str()
				.unwrap()
		);

		assert_eq!(
			"1024",
			response
				.headers()
				.get(CONTENT_LENGTH)
				.unwrap()
				.to_str()
				.unwrap()
		);

		assert!(response.headers().get(CONTENT_ENCODING).is_none());

		dbg!(response.headers());

		// ----------

		let request = Request::get("/files/html_32m.html")
			.header(ACCEPT_ENCODING, "gzip")
			.header(RANGE, "bytes=-1024")
			.body(Empty::default())
			.unwrap();

		let response = static_files.call(request).await.unwrap();
		assert_eq!(StatusCode::PARTIAL_CONTENT, response.status());
		assert_eq!(
			mime::TEXT_HTML.as_ref(),
			response
				.headers()
				.get(CONTENT_TYPE)
				.unwrap()
				.to_str()
				.unwrap()
		);

		assert_eq!(
			"1024",
			response
				.headers()
				.get(CONTENT_LENGTH)
				.unwrap()
				.to_str()
				.unwrap()
		);

		// html_32m.html does have an encoded copy.
		assert_eq!(
			"gzip",
			response
				.headers()
				.get(CONTENT_ENCODING)
				.unwrap()
				.to_str()
				.unwrap()
		);

		dbg!(response.headers());

		// -------------------------

		let request = Request::get("/files/text_16m.txt")
			.body(Empty::default())
			.unwrap();

		let response = static_files.call(request).await.unwrap();
		assert_eq!(StatusCode::OK, response.status());
		assert_eq!(
			mime::TEXT_PLAIN.as_ref(),
			response
				.headers()
				.get(CONTENT_TYPE)
				.unwrap()
				.to_str()
				.unwrap()
		);

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
			contents_16m.len().to_string(),
			response
				.headers()
				.get(CONTENT_LENGTH)
				.unwrap()
				.to_str()
				.unwrap()
		);

		dbg!(response.headers());

		// ----------

		let request = Request::get("/files/text_16m.txt")
			.header(ACCEPT_ENCODING, "gzip")
			.body(Empty::default())
			.unwrap();

		let response = static_files.call(request).await.unwrap();
		assert_eq!(StatusCode::OK, response.status());
		assert_eq!(
			mime::TEXT_PLAIN.as_ref(),
			response
				.headers()
				.get(CONTENT_TYPE)
				.unwrap()
				.to_str()
				.unwrap()
		);

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
			contents_16m.len().to_string(),
			response
				.headers()
				.get(CONTENT_LENGTH)
				.unwrap()
				.to_str()
				.unwrap()
		);

		// text_16m.txt doesn't have an encoded copy.
		assert!(response.headers().get(CONTENT_ENCODING).is_none());

		dbg!(response.headers());

		// -------------------------

		let request = Request::get("/files/small/html_8m.html")
			.body(Empty::default())
			.unwrap();

		let response = static_files.call(request).await.unwrap();
		assert_eq!(StatusCode::OK, response.status());
		assert_eq!(
			mime::TEXT_HTML.as_ref(),
			response
				.headers()
				.get(CONTENT_TYPE)
				.unwrap()
				.to_str()
				.unwrap()
		);

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
			contents_8m.len().to_string(),
			response
				.headers()
				.get(CONTENT_LENGTH)
				.unwrap()
				.to_str()
				.unwrap()
		);

		assert!(response.headers().get(CONTENT_RANGE).is_none());
		assert!(response.headers().get(CONTENT_ENCODING).is_none());

		dbg!(response.headers());

		assert_eq!(
			contents_8m.len(),
			response.collect().await.unwrap().to_bytes().len()
		);

		// ----------

		let request = Request::get("/files/small/html_8m.html")
			.header(ACCEPT_ENCODING, "gzip")
			.body(Empty::default())
			.unwrap();

		let response = static_files.call(request).await.unwrap();
		assert_eq!(StatusCode::OK, response.status());
		assert_eq!(
			mime::TEXT_HTML.as_ref(),
			response
				.headers()
				.get(CONTENT_TYPE)
				.unwrap()
				.to_str()
				.unwrap()
		);

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
			contents_8m.len().to_string(),
			response
				.headers()
				.get(CONTENT_LENGTH)
				.unwrap()
				.to_str()
				.unwrap()
		);

		assert!(response.headers().get(CONTENT_RANGE).is_none());
		assert_eq!(
			"gzip",
			response
				.headers()
				.get(CONTENT_ENCODING)
				.unwrap()
				.to_str()
				.unwrap()
		);

		dbg!(response.headers());

		assert_eq!(
			contents_8m.len(),
			response.collect().await.unwrap().to_bytes().len()
		);

		// -------------------------

		let request = Request::get("/files/small/html_8m.html")
			.header(RANGE, "bytes=0-512, 256-1024, -1024")
			.body(Empty::default())
			.unwrap();

		let response = static_files.call(request).await.unwrap();
		assert_eq!(StatusCode::PARTIAL_CONTENT, response.status());
		assert!(response
			.headers()
			.get(CONTENT_TYPE)
			.unwrap()
			.to_str()
			.unwrap()
			.starts_with("multipart/byteranges; boundary="));

		let content_length = response
			.headers()
			.get(CONTENT_LENGTH)
			.unwrap()
			.to_str()
			.unwrap()
			.parse::<usize>()
			.unwrap();

		assert!(content_length > 2048 && content_length < 8 * 1024);

		dbg!(response.headers());

		// ----------

		let request = Request::get("/files/small/html_8m.html")
			.header(ACCEPT_ENCODING, "gzip")
			.header(RANGE, "bytes=0-512, 256-1024, -1024")
			.body(Empty::default())
			.unwrap();

		let response = static_files.call(request).await.unwrap();
		assert_eq!(StatusCode::PARTIAL_CONTENT, response.status());

		assert!(response
			.headers()
			.get(CONTENT_TYPE)
			.unwrap()
			.to_str()
			.unwrap()
			.starts_with("multipart/byteranges; boundary="));

		let content_length = response
			.headers()
			.get(CONTENT_LENGTH)
			.unwrap()
			.to_str()
			.unwrap()
			.parse::<usize>()
			.unwrap();

		assert!(content_length > 2048 && content_length < 8 * 1024);

		// html_4m.html does have an encoded copy.
		assert_eq!(
			"gzip",
			response
				.headers()
				.get(CONTENT_ENCODING)
				.unwrap()
				.to_str()
				.unwrap()
		);

		dbg!(response.headers());

		// -------------------------

		let request = Request::get("/files/small/html_4m.html")
			.header(RANGE, "bytes=-1024")
			.body(Empty::default())
			.unwrap();

		let response = static_files.call(request).await.unwrap();
		assert_eq!(StatusCode::PARTIAL_CONTENT, response.status());
		assert_eq!(
			mime::TEXT_HTML.as_ref(),
			response
				.headers()
				.get(CONTENT_TYPE)
				.unwrap()
				.to_str()
				.unwrap()
		);

		assert_eq!(
			"1024",
			response
				.headers()
				.get(CONTENT_LENGTH)
				.unwrap()
				.to_str()
				.unwrap()
		);

		assert_eq!(
			"bytes 4193280-4194303/4194304",
			response
				.headers()
				.get(CONTENT_RANGE)
				.unwrap()
				.to_str()
				.unwrap()
		);

		assert!(response.headers().get(CONTENT_ENCODING).is_none());

		dbg!(response.headers());

		// ----------

		let request = Request::get("/files/small/html_4m.html")
			.header(ACCEPT_ENCODING, "gzip")
			.header(RANGE, "bytes=-1024")
			.body(Empty::default())
			.unwrap();

		let response = static_files.call(request).await.unwrap();
		assert_eq!(StatusCode::OK, response.status());
		assert_eq!(
			mime::TEXT_HTML.as_ref(),
			response
				.headers()
				.get(CONTENT_TYPE)
				.unwrap()
				.to_str()
				.unwrap()
		);

		assert!(response.headers().get(CONTENT_LENGTH).is_none());
		assert!(response.headers().get(CONTENT_RANGE).is_none());
		assert_eq!(
			"gzip",
			response
				.headers()
				.get(CONTENT_ENCODING)
				.unwrap()
				.to_str()
				.unwrap()
		);

		dbg!(response.headers());

		assert!(response.collect().await.unwrap().to_bytes().len() < contents_8m.len());

		// -------------------------

		let request = Request::get("/files/small/html_2m.html")
			.body(Empty::default())
			.unwrap();

		let response = static_files.call(request).await.unwrap();
		assert_eq!(StatusCode::OK, response.status());
		assert_eq!(
			mime::TEXT_HTML.as_ref(),
			response
				.headers()
				.get(CONTENT_TYPE)
				.unwrap()
				.to_str()
				.unwrap()
		);

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
			contents_2m.len().to_string(),
			response
				.headers()
				.get(CONTENT_LENGTH)
				.unwrap()
				.to_str()
				.unwrap()
		);

		assert!(response.headers().get(CONTENT_RANGE).is_none());
		assert!(response.headers().get(CONTENT_ENCODING).is_none());

		dbg!(response.headers());

		assert_eq!(
			contents_2m.len(),
			response.collect().await.unwrap().to_bytes().len()
		);

		// ----------

		let request = Request::get("/files/small/html_2m.html")
			.header(ACCEPT_ENCODING, "gzip")
			.body(Empty::default())
			.unwrap();

		let response = static_files.call(request).await.unwrap();
		assert_eq!(StatusCode::OK, response.status());
		assert_eq!(
			mime::TEXT_HTML.as_ref(),
			response
				.headers()
				.get(CONTENT_TYPE)
				.unwrap()
				.to_str()
				.unwrap()
		);

		assert!(response.headers().get(CONTENT_LENGTH).is_none());
		assert!(response.headers().get(CONTENT_RANGE).is_none());
		assert_eq!(
			"gzip",
			response
				.headers()
				.get(CONTENT_ENCODING)
				.unwrap()
				.to_str()
				.unwrap()
		);

		dbg!(response.headers());

		// Dynamically encoded body.
		let body = response.collect().await.unwrap().to_bytes();
		assert!(body.len() < contents_2m.len());
	}
}
