use std::{future::Future, pin::Pin};

pub use either::Either;

use super::pattern::Pattern;

// --------------------------------------------------

pub type BoxedError = Box<dyn std::error::Error + Send + Sync>;
pub type BoxedFuture<T> = Pin<Box<dyn Future<Output = T>>>;

// --------------------------------------------------

pub(crate) struct RouteSegments<'req> {
	route: &'req str,
	remaining_segments_index: usize,
}

impl<'req> RouteSegments<'req> {
	#[inline]
	pub(crate) fn new(route: &'req str) -> RouteSegments<'_> {
		// Route must contain at least a slash or must begin with one.
		Self {
			route,
			remaining_segments_index: 0,
		}
	}

	#[inline]
	pub(crate) fn has_remaining_segments(&self) -> bool {
		!self.route.is_empty()
	}

	#[inline]
	pub(crate) fn remaining_segments(&self) -> Option<&'req str> {
		if self.remaining_segments_index == self.route.len() {
			return None;
		}

		Some(&self.route[self.remaining_segments_index..])
	}

	#[inline]
	pub(crate) fn revert_to_segment(&mut self, segment: RouteSegment) {
		self.remaining_segments_index = segment.index;
	}
}

impl<'req> Iterator for RouteSegments<'req> {
	type Item = RouteSegment<'req>;

	#[inline]
	fn next(&mut self) -> Option<Self::Item> {
		if self.remaining_segments_index == self.route.len() {
			return None;
		}

		let next_segment_start_index = self.remaining_segments_index;
		let remaining_segments = &self.route[self.remaining_segments_index + 1..];

		let Some(next_segment_end_index) = remaining_segments.find('/') else {
			self.remaining_segments_index = self.route.len();

			return Some(RouteSegment {
				value: remaining_segments,
				index: next_segment_start_index,
			});
		};

		self.remaining_segments_index += next_segment_end_index;
		let next_segment = &remaining_segments[..next_segment_end_index];

		Some(RouteSegment {
			value: next_segment,
			index: next_segment_start_index,
		})
	}
}

pub(crate) struct RouteSegment<'req> {
	value: &'req str,
	index: usize,
}

impl<'req> RouteSegment<'req> {
	pub(crate) fn as_str(&self) -> &'req str {
		self.value
	}
}
//
// --------------------------------------------------------------------------------

pub(crate) fn patterns_to_string(patterns: &Vec<Pattern>) -> String {
	let mut string = String::new();
	for pattern in patterns {
		string.push('/');
		string.push_str(&pattern.to_string());
	}

	string
}
