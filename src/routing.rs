use crate::{request::Request, resource::ResourceService};

pub use hyper::Method;
pub use hyper::StatusCode;

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

pub(crate) struct RoutingState {
	pub(crate) path_traversal: RouteTraversal,
	pub(crate) current_resource: Option<ResourceService>,

	pub(crate) subtree_handler_exists: bool,
}

impl RoutingState {
	pub(crate) fn new(
		path_traversal: RouteTraversal,
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

pub(crate) struct RouteTraversal(usize);

impl RouteTraversal {
	#[inline]
	pub(crate) fn new() -> RouteTraversal {
		// Route must contain at least a slash or must begin with one.
		Self(0)
	}

	#[inline]
	pub(crate) fn has_remaining_segments(&self, route: &str) -> bool {
		self.0 < route.len()
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

	// #[inline]
	// pub(crate) fn ends_with_trailing_slash(&self, route: &str) -> bool {
	// 	route != "/" && route.as_bytes().last().unwrap() == &b'/'
	// }

	#[inline]
	pub(crate) fn next_segment<'req>(&mut self, route: &'req str) -> Option<(&'req str, usize)> {
		if self.0 == route.len() {
			return None;
		}

		let next_segment_start_index = self.0;
		let remaining_segments = &route[self.0 + 1..];
		println!(
			"next segment index: {}, remaining segments: {}",
			next_segment_start_index, remaining_segments
		);

		if let Some(next_segment_end_index) = remaining_segments.find('/') {
			self.0 += next_segment_end_index + 1;
			let next_segment = &remaining_segments[..next_segment_end_index];

			Some((next_segment, next_segment_start_index))
		} else {
			self.0 = route.len();

			Some((remaining_segments, next_segment_start_index))
		}
	}
}

// -------------------------

pub(crate) struct RouteSegments<'r> {
	route: &'r str,
	route_traversal: RouteTraversal,
}

impl<'r> RouteSegments<'r> {
	pub(crate) fn new(route: &'r str) -> RouteSegments<'r> {
		Self {
			route,
			route_traversal: RouteTraversal::new(),
		}
	}

	pub(crate) fn has_remaining_segments(&self) -> bool {
		self.route_traversal.has_remaining_segments(self.route)
	}

	pub(crate) fn revert_to_segment(&mut self, segment_index: usize) {
		self.route_traversal.revert_to_segment(segment_index);
	}
}

impl<'r> Iterator for RouteSegments<'r> {
	type Item = (&'r str, usize);

	fn next(&mut self) -> Option<Self::Item> {
		let Some((segment, segment_index)) = self.route_traversal.next_segment(self.route) else {
			return None;
		};

		Some((segment, segment_index))
	}
}

// --------------------------------------------------

pub(crate) struct UnusedRequest(Request);

impl From<Request> for UnusedRequest {
	#[inline]
	fn from(value: Request) -> Self {
		UnusedRequest(value)
	}
}

impl From<UnusedRequest> for Request {
	#[inline]
	fn from(value: UnusedRequest) -> Self {
		value.0
	}
}

impl AsRef<Request> for UnusedRequest {
	#[inline]
	fn as_ref(&self) -> &Request {
		&self.0
	}
}

impl AsMut<Request> for UnusedRequest {
	#[inline]
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

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

#[cfg(test)]
mod test {
	use super::*;

	#[test]
	fn route_traversal() {
		let mut route_traversal = RouteTraversal::new();

		let route = ["/abc", "/$regex_name:@capture_name(pattern)", "/*wildcard"];
		let route_segments = [
			(&route[0][1..], 0),
			(&route[1][1..], route[0].len()),
			(&route[2][1..], route[0].len() + route[1].len()),
		];

		let route_str = route.concat();
		println!("route str: {}", &route_str);

		for segment in route_segments {
			assert_eq!(segment, route_traversal.next_segment(&route_str).unwrap());
		}

		route_traversal.revert_to_segment(route_segments[1].1);

		assert!(route_traversal.has_remaining_segments(&route_str));
		assert_eq!(
			route[1].to_owned() + route[2],
			route_traversal.remaining_segments(&route_str).unwrap()
		);
	}
}
