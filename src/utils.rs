use std::{
	future::Future,
	pin::Pin,
	task::{Context, Poll},
	time::Duration,
};

use hyper::rt::Sleep;

use crate::{pattern::Pattern, routing::RouteSegments};

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

pub type BoxedError = Box<dyn std::error::Error + Send + Sync>;
pub type BoxedFuture<T> = Pin<Box<dyn Future<Output = T>>>;

// --------------------------------------------------------------------------------

pub(crate) mod mark {
	pub trait Sealed {}

	pub struct Private;
}

// --------------------------------------------------------------------------------

pub(crate) fn patterns_to_route(patterns: &[Pattern]) -> String {
	let mut string = String::new();
	for pattern in patterns {
		string.push('/');
		string.push_str(&pattern.to_string());
	}

	string
}

pub(crate) fn route_to_patterns(patterns: &str) -> Vec<Pattern> {
	let route_segments = RouteSegments::new(patterns);
	let mut patterns = Vec::new();
	for (segment, _) in route_segments {
		let pattern = Pattern::parse(segment);
		patterns.push(pattern);
	}

	patterns
}

// --------------------------------------------------------------------------------

pub(crate) struct Uncloneable<T>(Option<T>);

impl<T> Clone for Uncloneable<T> {
	fn clone(&self) -> Self {
		Self(None)
	}
}

impl<T> From<T> for Uncloneable<T> {
	fn from(value: T) -> Self {
		Self(Some(value))
	}
}

impl<T> Uncloneable<T> {
	pub(crate) fn as_ref(&self) -> Option<&T> {
		self.0.as_ref()
	}

	pub(crate) fn as_mut(&mut self) -> Option<&mut T> {
		self.0.as_mut()
	}

	pub(crate) fn into_inner(mut self) -> Option<T> {
		self.0.take()
	}
}

// --------------------------------------------------------------------------------

pub(crate) trait Uint {}

impl Uint for u8 {}
impl Uint for u16 {}
impl Uint for u32 {}
impl Uint for u64 {}

// --------------------------------------------------------------------------------

pub(crate) struct Interval {
	sleep: Pin<Box<dyn Sleep>>,
}

impl Interval {
	pub(crate) fn new(_duration: Duration) -> Self {
		todo!()
	}

	pub(crate) fn restart(&mut self) {
		todo!()
	}

	pub(crate) fn restart_with_duration(&mut self, _duration: Duration) {
		todo!()
	}

	pub(crate) fn pin(&mut self) -> Pin<&mut Self> {
		Pin::new(self)
	}
}

impl Future for Interval {
	type Output = ();

	fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
		match self.sleep.as_mut().poll(cx) {
			Poll::Ready(_) => {
				self.restart();

				Poll::Ready(())
			}
			Poll::Pending => Poll::Pending,
		}
	}
}

// --------------------------------------------------------------------------------
