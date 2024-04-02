use std::{borrow::Cow, str::Utf8Error};

use percent_encoding::percent_decode_str;

use crate::{pattern::ParamsList, request::Request, resource::ResourceService};

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

#[derive(Default)]
pub(crate) struct RoutingState {
	pub(crate) route_traversal: RouteTraversal,
	pub(crate) uri_params: ParamsList,
	pub(crate) subtree_handler_exists: bool,
}

impl RoutingState {
	#[inline(always)]
	pub(crate) fn new(route_traversal: RouteTraversal) -> RoutingState {
		Self {
			route_traversal,
			uri_params: ParamsList::new(),
			subtree_handler_exists: false,
		}
	}
}

// --------------------------------------------------

#[derive(Debug, Default)]
pub(crate) struct RouteTraversal(usize);

impl RouteTraversal {
	#[inline(always)]
	pub(crate) fn for_route(route: &str) -> RouteTraversal {
		if route.starts_with('/') {
			Self(1)
		} else {
			Self(0)
		}
	}

	#[inline(always)]
	pub(crate) fn has_remaining_segments(&self, route: &str) -> bool {
		self.0 < route.len()
	}

	#[inline(always)]
	pub(crate) fn remaining_segments<'req>(&self, route: &'req str) -> &'req str {
		if self.0 < route.len() {
			return &route[self.0..];
		}

		""
	}

	#[inline(always)]
	pub(crate) fn revert_to_segment(&mut self, segment_index: usize) {
		self.0 = segment_index;
	}

	#[inline(always)]
	pub(crate) fn ends_with_slash(&self, route: &str) -> bool {
		route != "/" && route.as_bytes().last().unwrap() == &b'/'
	}

	pub(crate) fn next_segment_index(&self) -> usize {
		self.0
	}

	pub(crate) fn next_segment<'req>(&mut self, route: &'req str) -> Option<(&'req str, usize)> {
		if self.0 < route.len() {
			let next_segment_start_index = self.0;
			let remaining_segments = &route[next_segment_start_index..];

			if let Some(next_segment_end_index) = remaining_segments.find('/') {
				self.0 += next_segment_end_index + 1;
				let next_segment = &remaining_segments[..next_segment_end_index];

				return Some((next_segment, next_segment_start_index));
			}

			self.0 = route.len();

			return Some((remaining_segments, next_segment_start_index));
		}

		None
	}

	pub(crate) fn next_segment_decoded<'req>(
		&mut self,
		route: &'req str,
	) -> Option<(Result<Cow<'req, str>, Utf8Error>, usize)> {
		self
			.next_segment(route)
			.map(|(segment, index)| (percent_decode_str(segment).decode_utf8(), index))
	}
}

// -------------------------

#[derive(Debug)]
pub(crate) struct RouteSegments<'r> {
	route: &'r str,
	route_traversal: RouteTraversal,
}

impl<'r> RouteSegments<'r> {
	pub(crate) fn new(route: &'r str) -> RouteSegments<'r> {
		Self {
			route,
			route_traversal: RouteTraversal::for_route(route),
		}
	}

	pub(crate) fn has_remaining_segments(&self) -> bool {
		self.route_traversal.has_remaining_segments(self.route)
	}

	pub(crate) fn revert_to_segment(&mut self, segment_index: usize) {
		self.route_traversal.revert_to_segment(segment_index);
	}

	pub(crate) fn ends_with_slash(&self) -> bool {
		self.route_traversal.ends_with_slash(self.route)
	}
}

impl<'r> Iterator for RouteSegments<'r> {
	type Item = (&'r str, usize);

	fn next(&mut self) -> Option<Self::Item> {
		let (segment, segment_index) = self.route_traversal.next_segment(self.route)?;

		Some((segment, segment_index))
	}
}

// --------------------------------------------------

pub(crate) struct UnusedRequest(Request);

impl UnusedRequest {
	#[inline]
	pub(crate) fn into_request(self) -> Request {
		self.0
	}
}

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

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

#[cfg(test)]
mod test {
	use super::*;

	#[test]
	fn route_traversal() {
		let route = ["/abc", "/{capture_name:pattern}", "/{wildcard}"];
		let route_segments = [
			(&route[0][1..], 1),
			(&route[1][1..], route[0].len() + 1),
			(&route[2][1..], route[0].len() + route[1].len() + 1),
		];

		let route_str = route.concat();
		println!("route str: {}", &route_str);

		let mut route_traversal = RouteTraversal::for_route(&route_str);

		for segment in route_segments {
			assert_eq!(segment, route_traversal.next_segment(&route_str).unwrap());
		}

		route_traversal.revert_to_segment(route_segments[1].1);

		assert!(route_traversal.has_remaining_segments(&route_str));
		assert_eq!(
			route[1][1..].to_owned() + route[2],
			route_traversal.remaining_segments(&route_str)
		);
	}
}
