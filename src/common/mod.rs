use std::{
	any::Any,
	fmt::Display,
	future::Future,
	io::ErrorKind,
	pin::Pin,
	task::{Context, Poll},
	time::Duration,
};

use http::StatusCode;

use crate::{
	handler::BoxedHandler,
	pattern::Pattern,
	response::{ErrorResponse, IntoResponse, Response},
	routing::RouteSegments,
};

// --------------------------------------------------

#[macro_use]
pub(crate) mod macros;

pub(crate) mod timer;

#[cfg(test)]
pub(crate) mod test_helpers;

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

pub type BoxedError = Box<dyn std::error::Error + Send + Sync>;
pub type BoxedFuture<T> = Pin<Box<dyn Future<Output = T>>>;

// --------------------------------------------------------------------------------

// Used when expecting a valid value in Options or Results.
pub(crate) const SCOPE_VALIDITY: &'static str = "scope validity";

// --------------------------------------------------------------------------------

pub trait IntoArray<T, const N: usize> {
	fn into_array(self) -> [T; N];
}

impl<T, const N: usize> IntoArray<T, N> for [T; N]
where
	T: IntoArray<T, 1>,
{
	fn into_array(self) -> [T; N] {
		self
	}
}

// --------------------------------------------------------------------------------

pub(crate) mod mark {
	pub trait Sealed {}

	// ----------

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
// Uncloneable

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
// MaybeBoxed

#[derive(Clone)]
pub(crate) enum MaybeBoxed<H> {
	Boxed(BoxedHandler),
	Unboxed(H),
}

// --------------------------------------------------------------------------------

#[inline]
pub(crate) fn strip_double_quotes(slice: &[u8]) -> &[u8] {
	let slice = if let Some(stripped_slice) = slice.strip_prefix(b"\"") {
		stripped_slice
	} else {
		slice
	};

	if let Some(stripped_slice) = slice.strip_suffix(b"\"") {
		stripped_slice
	} else {
		slice
	}
}

// --------------------------------------------------------------------------------

pub(crate) struct Deferred<Func: FnMut()>(Func);

impl<Func: FnMut()> Deferred<Func> {
	pub(crate) fn call(func: Func) -> Self {
		Self(func)
	}
}

impl<Func: FnMut()> Drop for Deferred<Func> {
	fn drop(&mut self) {
		(self.0)()
	}
}

// --------------------------------------------------------------------------------
