use std::{fs::Metadata, io::Error as IoError, path::Path, str::FromStr, sync::Arc};

use http::{
	header::{IF_MATCH, IF_MODIFIED_SINCE, IF_NONE_MATCH, IF_RANGE, IF_UNMODIFIED_SINCE, RANGE},
	HeaderMap, Method, StatusCode,
};
use httpdate::HttpDate;

use crate::{
	handler::{get, request_handlers::handle_misdirected_request},
	request::Request,
	response::{stream::FileStream, IntoResponse, Response},
	routing::RoutingState,
	utils::{strip_double_quotes, BoxedError, Uncloneable},
};

use super::Resource;

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

pub struct StaticFiles {
	some_resource: Option<Resource>,
	some_files_dir: Option<Arc<Path>>,
	some_tagger: Option<Arc<dyn Tagger>>,
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
			flags: Flags::GET,
		}
	}

	pub fn with_tagger(mut self, tagger: Arc<dyn Tagger>) -> Self {
		self.some_tagger = Some(tagger);

		self
	}

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

		let get_handler = move |request: Request| {
			let files_dir = files_dir.clone();
			let some_hash_storage = some_hash_storage.clone();

			get_handler(request, files_dir, some_hash_storage, attachments)
		};

		if self.flags.has(Flags::GET) {
			resource.set_handler_for(get(get_handler));
		}

		resource
	}
}

// -------------------------

pub trait Tagger: Send + Sync {
	fn tag(&self, path: &Path) -> Result<Arc<str>, BoxedError>;
}

// -------------------------

bit_flags! {
	Flags: u8 {
		ATTACHMENTS = 0b0001;
		GET = 0b0010;
		POST = 0b0100;
		DELETE = 0b1000;
	}
}

// -------------------------

// ???
#[derive(Debug)]
pub enum FileServiceError {
	IoError(IoError),
	NonDirPath,
}

impl From<IoError> for FileServiceError {
	fn from(error: IoError) -> Self {
		Self::IoError(error)
	}
}

// --------------------------------------------------

#[inline(always)]
async fn get_handler(
	request: Request,
	files_dir: Arc<Path>,
	some_hash_storage: Option<Arc<dyn Tagger>>,
	attachments: bool,
) -> Result<Response, Response> {
	let request_path = request.uri().path();

	let routing_state = request
		.extensions()
		.get::<Uncloneable<RoutingState>>()
		.expect("Uncloneable<RoutingState> should be inserted before routing starts")
		.as_ref()
		.expect("RoutingState should always exist in Uncloneable");

	let Some(remaining_segments) = routing_state
		.path_traversal
		.remaining_segments(request_path)
	else {
		return Err(handle_misdirected_request(request).await); // ???
	};

	let Ok(path) = files_dir.join(remaining_segments).canonicalize() else {
		return Err(handle_misdirected_request(request).await); // ???
	};

	// TODO: Test canonicalize.
	// if !file_path.starts_with(files_dir) {
	// 	return Err(misdirected_request_handler(request).await); // ???
	// }

	let path_metadata = match path.metadata() {
		Ok(metadata) => metadata,
		Err(error) => return Err(handle_misdirected_request(request).await), // ???
	};

	if !path_metadata.is_file() {
		return Err(handle_misdirected_request(request).await); // ???
	}

	match evaluate_preconditions(
		request.headers(),
		request.method(),
		some_hash_storage,
		&path,
		&path_metadata,
	) {
		Ok(Some(ranges)) => match FileStream::open_ranges(path, ranges, false) {
			Ok(file_stream) => Ok(file_stream.into_response()),
			Err(error) => {
				todo!()
			}
		},
		Ok(None) => match FileStream::open(path) {
			Ok(file_stream) => Ok(file_stream.into_response()),
			Err(error) => {
				todo!()
			}
		},
		Err(status_code) => Ok(status_code.into_response()),
	}
}

// ----------

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

// --------------------------------------------------------------------------------
