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
}

impl<'req> Iterator for PathSegments<'req> {
	type Item = &'req str;

	#[inline]
	fn next(&mut self) -> Option<Self::Item> {
		if self.remaining_segments_index == self.path.len() {
			return None
		}

		let remaining_segments = &self.path[self.remaining_segments_index + 1..];

		let Some(next_segment_end_index) = remaining_segments.find('/') else {
			self.remaining_segments_index = self.path.len();

			return Some(remaining_segments)
		};

		self.remaining_segments_index += next_segment_end_index;
		let next_segment = &remaining_segments[..next_segment_end_index];

		Some(next_segment)
	}
}

