use super::resource::Resource;

// --------------------------------------------------

pub type Request = hyper::Request<hyper::Body>;
pub type Response = hyper::Response<hyper::Body>;
pub type BoxedError = tower::BoxError;
pub type BoxPinnedFuture = futures::future::BoxFuture<'static, Result<Response, BoxedError>>;
pub type BoxedService = Box<
	dyn tower::Service<Request, Response = Response, Error = BoxedError, Future = BoxPinnedFuture>
		+ Send
		+ Sync,
>;

// --------------------------------------------------

pub(crate) struct RoutingState<'req> {
	pub(crate) path_segments: PathSegments<'req>,
	pub(crate) current_resource: Option<&'req Resource>,

	pub(crate) subtree_handler_exists: bool,
}

// --------------------------------------------------

pub(crate) struct UnusedRequest(Request);

impl From<Request> for UnusedRequest {
	fn from(value: Request) -> Self {
		UnusedRequest(value)
	}
}

impl From<UnusedRequest> for Request {
	fn from(value: UnusedRequest) -> Self {
		value.0	
	}
}

impl AsRef<Request> for UnusedRequest {
	fn as_ref(&self) -> &Request {
		&self.0
	}
}

impl AsMut<Request> for UnusedRequest {
	fn as_mut(&mut self) -> &mut Request {
		&mut self.0
	}
}

// --------------------------------------------------

// PathSegments is an iterator over path segments. It's valid for the life time of the request.
pub(crate) struct PathSegments<'req> {
	path: &'req str,
	remaining_segments_index: usize,
}

impl<'req> PathSegments<'req> {
	#[inline]
	pub(crate) fn new(path: &'req str) -> PathSegments<'_> {
		// Path contains at least a slash or begins with one.
		Self{path, remaining_segments_index: 0}
	}

	#[inline]
	pub(crate) fn has_remaining_segments(&self) -> bool {
		!self.path.is_empty()
	}
	
	#[inline]
	pub(crate) fn revert_to_segment(&mut self, segment: PathSegment) {
		self.remaining_segments_index = segment.index;
	}
}

impl<'req> Iterator for PathSegments<'req> {
	type Item = PathSegment<'req>;

	#[inline]
	fn next(&mut self) -> Option<Self::Item> {
		if self.remaining_segments_index == self.path.len() {
			return None
		}

		let next_segment_start_index = self.remaining_segments_index;
		let remaining_segments = &self.path[self.remaining_segments_index + 1..];

		let Some(next_segment_end_index) = remaining_segments.find('/') else {
			self.remaining_segments_index = self.path.len();

			return Some(PathSegment{value: remaining_segments, index: next_segment_start_index})
		};

		self.remaining_segments_index += next_segment_end_index;
		let next_segment = &remaining_segments[..next_segment_end_index];

		Some(PathSegment{value: next_segment, index: next_segment_start_index})
	}
}

pub(crate) struct PathSegment<'req> {
	value: &'req str,
	index: usize,
}

impl<'req> PathSegment<'req> {
	pub(crate) fn as_str(&self) -> &'req str {
		self.value
	}
}

