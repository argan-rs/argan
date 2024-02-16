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
	request::Request,
	response::{stream::{ContentCoding, FileStream}, IntoResponse, Response},
	routing::RoutingState,
};

use super::Resource;

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

const UNCOMPRESSED: &'static str = "uncompressed";
const COMPRESSED: &'static str = "compressed";

// --------------------------------------------------

pub struct StaticFiles {
	some_resource: Option<Resource>,
	some_files_dir: Option<Box<Path>>,
	some_tagger: Option<Arc<dyn Tagger>>,
	min_size_to_compress: u64,
	max_size_to_compress: u64,
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
			min_size_to_compress: 1024,
			max_size_to_compress: 8 * 1024 * 1024,
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

	pub fn with_dynamic_compression(mut self) -> Self {
		self.flags.add(Flags::DYNAMIC_COMPRESSION);

		self
	}

	pub fn as_attachments(mut self) -> Self {
		self.flags.add(Flags::ATTACHMENTS);

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
		let dynamic_compression = self.flags.has(Flags::DYNAMIC_COMPRESSION);
		let attachments = self.flags.has(Flags::ATTACHMENTS);
		let min_size_to_compress = self.min_size_to_compress;
		let max_size_to_compress = self.max_size_to_compress;

		let get_handler = move |request: Request| {
			let files_dir = files_dir.clone();
			let some_hash_storage = some_hash_storage.clone();

			get_handler(
				request,
				files_dir,
				some_hash_storage,
				attachments,
				dynamic_compression,
				min_size_to_compress,
				max_size_to_compress,
			)
		};

		if self.flags.has(Flags::GET) {
			resource.set_handler(get(get_handler));
		}

		resource
	}
}

// -------------------------

/* pub */ trait Tagger: Send + Sync {
	fn tag(&self, path: &Path) -> Result<Arc<str>, BoxedError>;
}

// -------------------------

bit_flags! {
	#[derive(Default, Clone)]
	Flags: u8 {
		DYNAMIC_COMPRESSION = 0b0_0001;
		ATTACHMENTS = 0b0_0010;
		GET = 0b0_0100;
		POST = 0b0_1000;
		DELETE = 0b1_0000;
	}
}

// --------------------------------------------------

async fn get_handler(
	mut request: Request,
	files_dir: Box<Path>,
	some_hash_storage: Option<Arc<dyn Tagger>>,
	attachments: bool,
	dynamic_compression: bool,
	min_size_to_compress: u64,
	max_size_to_compress: u64,
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
		return Err(StaticFileError::NotFound);
	};

	// TODO: Canonicalize must be tested. We may need to implement it ourselves.
	let relative_path_to_file = AsRef::<Path>::as_ref(remaining_segments).canonicalize()?;

	let (coding, path_buf, should_compress) = evaluate_optimal_coding(
		request.headers(),
		files_dir.as_ref(),
		remaining_segments,
		dynamic_compression,
		min_size_to_compress,
		max_size_to_compress,
	)?;

	let path_metadata = match path_buf.metadata() {
		Ok(metadata) => metadata,
		Err(error) => return Err(StaticFileError::IoError(error)),
	};

	// if !path_metadata.is_file() {
	// 	return Err(StaticFileError::NotFound);
	// }

	if coding.is_empty() {
		match evaluate_preconditions(
			request.headers(),
			request.method(),
			some_hash_storage,
			&path_buf,
			&path_metadata,
		) {
			Ok(Some(ranges)) => match FileStream::open_ranges(path_buf, ranges, false) {
				Ok(file_stream) => Ok(file_stream.into_response()),
				Err(error) => {
					todo!()
				}
			},
			Ok(None) => match FileStream::open(path_buf, ContentCoding::Identity) {
				Ok(file_stream) => Ok(file_stream.into_response()),
				Err(error) => {
					todo!()
				}
			},
			Err(status_code) => Ok(status_code.into_response()),
		}
	} else {
		let content_coding = match coding {
			"gzip" => ContentCoding::Gzip(6), // TODO: Level.
			_ => ContentCoding::Identity,
		};

		match FileStream::open(path_buf, content_coding) {
			Ok(file_stream) => Ok(file_stream.into_response()),
			Err(error) => {
				todo!()
			}
		}
	}
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
					.join(COMPRESSED)
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
				path_buf.push(UNCOMPRESSED);
				path_buf.push(relative_path_to_file);

				path_buf
			} else {
				files_dir.join(UNCOMPRESSED).join(relative_path_to_file)
			};

			if !path_buf.is_file() {
				return Err(StaticFileError::NotFound);
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
				return Err(StaticFileError::AcceptEncoding("identity"));
			}

			let path_buf = files_dir.join(UNCOMPRESSED).join(relative_path_to_file);
			if !path_buf.is_file() {
				return Err(StaticFileError::NotFound);
			}

			/* return Err(StaticFileError::AcceptEncoding("br, gzip, identity")); */
			return Err(StaticFileError::AcceptEncoding("gzip, identity"));
		}

		if let Some(path_buf) = some_path_buf {
			return Ok(("", path_buf, false));
		}
	}

	let path_buf = files_dir.join(UNCOMPRESSED).join(relative_path_to_file);
	if !path_buf.is_file() {
		return Err(StaticFileError::NotFound);
	}

	Ok(("", path_buf, false))
}

fn evaluate_preconditions<'r>(
	request_headers: &'r HeaderMap,
	request_method: &Method,
	some_hash_storage: Option<Arc<dyn Tagger>>,
	path: &Path,
	path_metadata: &Metadata,
) -> Result<Option<&'r str>, StatusCode> {
	let mut some_file_hash = None;
	if let Some(hash_storage) = some_hash_storage.as_ref() {
		if let Some(hashes_to_match) = request_headers.get(IF_MATCH).map(|value| value.as_bytes()) {
			// step 1
			match hash_storage.tag(&path) {
				Ok(file_hash) => {
					if hashes_to_match != b"*" {
						if !hashes_to_match
							.split(|ch| *ch == b',')
							.any(|hash_to_match| file_hash.as_bytes() == strip_double_quotes(hash_to_match))
						{
							return Err(StatusCode::PRECONDITION_FAILED);
						}
					}

					some_file_hash = Some(file_hash);
				}
				Err(_) => return Err(StatusCode::NOT_FOUND), // No such file.
			}
		}
	}

	// The only way some_file_hash is Some is when If-Match exists.
	if some_file_hash.is_none() {
		if let Some(time_to_match) = request_headers
			.get(IF_UNMODIFIED_SINCE)
			.and_then(|value| value.to_str().ok())
		{
			// step 2
			let Ok(modified_time) = path_metadata.modified() else {
				return Err(StatusCode::INTERNAL_SERVER_ERROR); // ???
			};

			let Ok(http_date_to_match) = HttpDate::from_str(time_to_match) else {
				return Err(StatusCode::BAD_REQUEST);
			};

			if HttpDate::from(modified_time) != http_date_to_match {
				return Err(StatusCode::PRECONDITION_FAILED);
			}
		}
	}

	let mut no_if_none_match_header = true;
	if let Some(hash_storage) = some_hash_storage.as_ref() {
		if let Some(hashes_to_match) = request_headers
			.get(IF_NONE_MATCH)
			.map(|value| value.as_bytes())
		{
			// step 3
			if some_file_hash.is_none() {
				some_file_hash = hash_storage.tag(&path).ok();
			};

			let unmodified = if let Some(file_hash) = some_file_hash.as_ref() {
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
				hashes_to_match != b"*"
			};

			if unmodified {
				if request_method == Method::GET || request_method == Method::HEAD {
					return Err(StatusCode::NOT_MODIFIED);
				} else {
					return Err(StatusCode::PRECONDITION_FAILED);
				}
			}

			no_if_none_match_header = false;
		}
	}

	if no_if_none_match_header {
		if request_method == Method::GET || request_method == Method::HEAD {
			// step 4
			if let Some(time_to_match) = request_headers
				.get(IF_MODIFIED_SINCE)
				.and_then(|value| value.to_str().ok())
			{
				let Ok(modified_time) = path_metadata.modified() else {
					return Err(StatusCode::INTERNAL_SERVER_ERROR); // ???
				};

				let Ok(http_date_to_match) = HttpDate::from_str(time_to_match) else {
					return Err(StatusCode::BAD_REQUEST);
				};

				if HttpDate::from(modified_time) == http_date_to_match {
					return Err(StatusCode::NOT_MODIFIED);
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
					return Ok(None);
				}

				if range_precondition.starts_with('"') {
					let Some(hash_storage) = some_hash_storage else {
						return Ok(None);
					};

					let hash_to_match = strip_double_quotes(range_precondition.as_bytes());

					let file_hash = if let Some(file_hash) = some_file_hash {
						file_hash
					} else {
						match hash_storage.tag(&path) {
							Ok(file_hash) => file_hash,
							Err(_) => return Err(StatusCode::NOT_FOUND),
						}
					};

					if file_hash.as_bytes() == hash_to_match {
						return Ok(some_ranges_str);
					}
				} else {
					let Ok(modified_time) = path_metadata.modified() else {
						return Err(StatusCode::INTERNAL_SERVER_ERROR); // ???
					};

					let Ok(http_date_to_match) = HttpDate::from_str(range_precondition) else {
						return Err(StatusCode::BAD_REQUEST);
					};

					if HttpDate::from(modified_time) == http_date_to_match {
						return Ok(some_ranges_str);
					}
				}
			} else {
				return Ok(some_ranges_str);
			}
		}
	}

	Ok(None)
}

// --------------------------------------------------

#[derive(Debug, crate::ImplError)]
pub enum StaticFileError {
	#[error(transparent)]
	InvalidAcceptEncoding(#[from] SplitHeaderValueError),
	#[error("file not found")]
	NotFound,
	#[error("Accept-Encoding must be {0}")]
	AcceptEncoding(&'static str),
	#[error(transparent)]
	IoError(#[from] IoError),
}

impl IntoResponse for StaticFileError {
	fn into_response(self) -> Response {
		let mut response = Response::default();

		match self {
			Self::InvalidAcceptEncoding(_) => *response.status_mut() = StatusCode::BAD_REQUEST,
			Self::NotFound => *response.status_mut() = StatusCode::NOT_FOUND,
			Self::AcceptEncoding(codings) => {
				response
					.headers_mut()
					.insert(ACCEPT_ENCODING, HeaderValue::from_static(codings));
			}
			Self::IoError(_) => *response.status_mut() = StatusCode::INTERNAL_SERVER_ERROR,
		}

		response
	}
}

// --------------------------------------------------------------------------------
