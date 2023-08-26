use std::{future::Future, pin::Pin};

use super::pattern::Pattern;

// --------------------------------------------------

pub type BoxedError = Box<dyn std::error::Error + Send + Sync>;
pub type BoxedFuture<T> = Pin<Box<dyn Future<Output = T>>>;

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

	pub(crate) fn revert_to_segment(&mut self, segment: RouteSegment) {
		self.traversal_state.revert_to_segment(segment.index);
	}
}

impl<'r> Iterator for RouteSegments<'r> {
	type Item = RouteSegment<'r>;

	fn next(&mut self) -> Option<Self::Item> {
		let Some((segment, segment_index)) = self.traversal_state.next_segment(self.route) else {
			return None;
		};

		Some(RouteSegment::new(segment, segment_index))
	}
}

// ----------

pub(crate) struct RouteSegment<'req> {
	value: &'req str,
	index: usize,
}

impl<'req> RouteSegment<'req> {
	pub(crate) fn new(segment: &'req str, segment_index: usize) -> Self {
		Self {
			value: segment,
			index: segment_index,
		}
	}

	pub(crate) fn as_str(&self) -> &'req str {
		self.value
	}
}

// --------------------------------------------------------------------------------

pub(crate) fn patterns_to_string(patterns: &Vec<Pattern>) -> String {
	let mut string = String::new();
	for pattern in patterns {
		string.push('/');
		string.push_str(&pattern.to_string());
	}

	string
}

// --------------------------------------------------------------------------------
