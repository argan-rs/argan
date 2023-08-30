use crate::{request::Request, resource::ResourceService};

pub use hyper::Method;
pub use hyper::StatusCode;

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

pub(crate) struct RoutingState {
	pub(crate) path_traversal: RouteTraversalState,
	pub(crate) current_resource: Option<ResourceService>,

	pub(crate) subtree_handler_exists: bool,
}

impl RoutingState {
	pub(crate) fn new(
		path_traversal: RouteTraversalState,
		resource_service: ResourceService,
	) -> RoutingState {
		Self {
			path_traversal,
			current_resource: Some(resource_service),
			subtree_handler_exists: false,
		}
	}
}

// --------------------------------------------------

pub(crate) struct RouteTraversalState(usize);

impl RouteTraversalState {
	#[inline]
	pub(crate) fn new() -> RouteTraversalState {
		// Route must contain at least a slash or must begin with one.
		Self(0)
	}

	#[inline]
	pub(crate) fn has_remaining_segments(&self, route: &str) -> bool {
		!route.is_empty()
	}

	#[inline]
	pub(crate) fn remaining_segments<'req>(&self, route: &'req str) -> Option<&'req str> {
		if self.0 == route.len() {
			return None;
		}

		Some(&route[self.0..])
	}

	#[inline]
	pub(crate) fn revert_to_segment(&mut self, segment_index: usize) {
		self.0 = segment_index;
	}

	#[inline]
	pub(crate) fn ends_with_trailing_slash(&self, route: &str) -> bool {
		route != "/" && route.as_bytes().last().unwrap() == &b'/'
	}

	#[inline]
	pub(crate) fn next_segment<'req>(&mut self, route: &'req str) -> Option<(&'req str, usize)> {
		if self.0 == route.len() {
			return None;
		}

		let next_segment_start_index = self.0;
		let remaining_segments = &route[self.0 + 1..];

		let Some(next_segment_end_index) = remaining_segments.find('/') else {
			self.0 = route.len();

			return Some((remaining_segments, next_segment_start_index));
		};

		self.0 += next_segment_end_index;
		let next_segment = &remaining_segments[..next_segment_end_index];

		Some((next_segment, next_segment_start_index))
	}
}

// -------------------------

pub(crate) struct RouteSegments<'r> {
	route: &'r str,
	traversal_state: RouteTraversalState,
}

impl<'r> RouteSegments<'r> {
	pub(crate) fn new(route: &'r str) -> RouteSegments<'r> {
		Self {
			route,
			traversal_state: RouteTraversalState::new(),
		}
	}

	pub(crate) fn has_remaining_segments(&self) -> bool {
		self.traversal_state.has_remaining_segments(self.route)
	}

	pub(crate) fn revert_to_segment(&mut self, segment_index: usize) {
		self.traversal_state.revert_to_segment(segment_index);
	}
}

impl<'r> Iterator for RouteSegments<'r> {
	type Item = (&'r str, usize);

	fn next(&mut self) -> Option<Self::Item> {
		let Some((segment, segment_index)) = self.traversal_state.next_segment(self.route) else {
			return None;
		};

		Some((segment, segment_index))
	}
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

impl UnusedRequest {
	#[inline]
	pub(crate) fn into_request(self) -> Request {
		self.0
	}
}
