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
pub(crate) struct PathSegments<'req>(&'req str);

impl<'req> PathSegments<'req> {
	#[inline]
	pub(crate) fn new(path: &'req str) -> PathSegments<'_> {
		Self(path)
	}

	#[inline]
	pub(crate) fn has_no_remaining(&self) -> bool {
		self.0.is_empty()
	}

	#[inline]
	pub(crate) fn ends_with_trailing_slash(&self) -> bool {
		self.0 != "/" && self.0.ends_with('/')
	}
}

impl<'req> Iterator for PathSegments<'req> {
	type Item = &'req str;

	#[inline]
	fn next(&mut self) -> Option<Self::Item> {
		if self.0.is_empty() {
			return None;
		}

		let Some((next_segment, remaining)) = self.0.split_once('/') else {
			let last_segment = self.0;
			self.0 = "";

			return Some(last_segment);
		};

		self.0 = remaining;

		if next_segment.is_empty() {
			return Some("/");
		}

		Some(next_segment)
	}
}
