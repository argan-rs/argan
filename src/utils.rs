use std::{future::Future, pin::Pin};

pub use either::Either;

// --------------------------------------------------

pub type BoxedError = Box<dyn std::error::Error + Send + Sync>;
pub type BoxedFuture<'a, T> = Pin<Box<dyn Future<Output = T> + 'a>>;

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
		Self {
			path,
			remaining_segments_index: 0,
		}
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
			return None;
		}

		let next_segment_start_index = self.remaining_segments_index;
		let remaining_segments = &self.path[self.remaining_segments_index + 1..];

		let Some(next_segment_end_index) = remaining_segments.find('/') else {
			self.remaining_segments_index = self.path.len();

			return Some(PathSegment {
				value: remaining_segments,
				index: next_segment_start_index,
			});
		};

		self.remaining_segments_index += next_segment_end_index;
		let next_segment = &remaining_segments[..next_segment_end_index];

		Some(PathSegment {
			value: next_segment,
			index: next_segment_start_index,
		})
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
