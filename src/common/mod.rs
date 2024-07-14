//! Common types and functions.

// ----------

use std::{borrow::Cow, net::SocketAddr};

use http::Extensions;

use crate::{handler::BoxedHandler, pattern::Pattern, request::routing::RouteSegments};

// ----------

pub use argan_core::{BoxedError, BoxedFuture};

// --------------------------------------------------

#[macro_use]
pub(crate) mod macros;

pub mod node_properties;
pub use node_properties::RequestExtensionsModifier;

#[cfg(any(feature = "private-cookies", feature = "signed-cookies"))]
pub use node_properties::NodeCookieKey;

pub(crate) mod header_utils;

#[cfg(all(test, feature = "full"))]
pub(crate) mod test_helpers;

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

// --------------------------------------------------
// IntoArray trait

#[doc(hidden)]
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

// --------------------------------------------------
// Markers

pub(crate) mod marker {
	pub trait Sealed {}

	// -------------------------

	pub struct Private;
}

// --------------------------------------------------

// Used when expecting a valid value in Options or Results.
pub(crate) const SCOPE_VALIDITY: &str = "scope validity";

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

pub(crate) fn trim(mut slice: &[u8]) -> &[u8] {
	if let Some(position) = slice.iter().position(|ch| !ch.is_ascii_whitespace()) {
		slice = &slice[position..];
		if let Some(position) = slice.iter().rev().position(|ch| !ch.is_ascii_whitespace()) {
			return &slice[..slice.len() - position];
		}
	};

	b""
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

// Eliminates each 'empty' and '.' segment.
// Eliminates each '..' segment with its 'non-..' parent.
// If there is no segment left, returns an empty string.
pub(crate) fn normalize_path(path: &str) -> String {
	let mut new_path = String::with_capacity(path.len() + 1);
	let mut segment_indices = vec![0];

	for segment in path.split('/') {
		if segment.is_empty() || segment == "." {
			continue;
		}

		if segment == ".." {
			let last_index = segment_indices.pop().unwrap_or(0);
			new_path.truncate(last_index);

			continue;
		}

		segment_indices.push(new_path.len());

		if !new_path.is_empty() || path.starts_with(['/', '.']) {
			new_path.push('/');
		}

		new_path.push_str(segment);
	}

	new_path
}

// --------------------------------------------------
// NodeExtensions

/// Extensions of a node ([`Router`](crate::Router), [`Resource`](crate::Resource)).
///
/// Carried by an [`Args`](crate::handler::Args) while the request is being routed.
#[derive(Clone)]
pub struct NodeExtensions<'n>(Cow<'n, Extensions>);

impl<'n> NodeExtensions<'n> {
	#[inline(always)]
	pub(crate) fn new_borrowed(extensions: &'n Extensions) -> Self {
		Self(Cow::Borrowed(extensions))
	}

	#[inline(always)]
	pub(crate) fn new_owned(extensions: Extensions) -> Self {
		Self(Cow::Owned(extensions))
	}

	/// Returns a reference to a value of type `T` if `NodeExtensions` contains it.
	#[inline(always)]
	pub fn get_ref<T: Send + Sync + 'static>(&self) -> Option<&T> {
		self.0.get::<T>()
	}

	/// Returns a value of type `T` if `NodeExtensions` contains it, removing
	/// it from extensions.
	///
	/// The method can be used when `'n: 'static`. This is true when a handler
	/// function receives `NodeExtensions` in [`Args<'static, Ext>`](crate::handler::Args).
	#[inline(always)]
	pub fn remove<T>(&mut self) -> Option<T>
	where
		'n: 'static,
		T: Send + Sync + 'static,
	{
		self.0.to_mut().remove::<T>()
	}

	#[inline(always)]
	pub(crate) fn into_owned(self) -> NodeExtensions<'static> {
		NodeExtensions(Cow::Owned(self.0.into_owned()))
	}
}

// --------------------------------------------------
// WithSocketAddr

#[doc(hidden)]
pub trait CloneWithPeerAddr: marker::Sealed {
	fn clone_with_peer_addr(&self, addr: SocketAddr) -> Self;
}

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

#[cfg(test)]
mod test {
	#[test]
	fn normalize_path() {
		let cases = [
			("a/b/c", "a/b/c"),
			("../a/b/c", "/a/b/c"),
			("/../a/b/c", "/a/b/c"),
			("/a/../b/c", "/b/c"),
			("/a/b/../c", "/a/c"),
			("/a/b/c/..", "/a/b"),
			("../../a/b/c", "/a/b/c"),
			("/../../a/b/c", "/a/b/c"),
			("/a/../../b/c", "/b/c"),
			("/a/b/../../c", "/c"),
			("/a/b/c/../..", "/a"),
			("../../../a/b/c", "/a/b/c"),
			("/../../../a/b/c", "/a/b/c"),
			("/a/../../../b/c", "/b/c"),
			("/a/b/../../../c", "/c"),
			("/a/b/c/../../..", ""),
			("../a/../b/c", "/b/c"),
			("/../a/../b/c", "/b/c"),
			("/a/../b/../c", "/c"),
			("/a/b/../c/..", "/a"),
			("/a/b/c/../..", "/a"),
			("///a/b/c/", "/a/b/c"),
			("/a///b/c/", "/a/b/c"),
			("/a/b///c/", "/a/b/c"),
			("/a/b/c///", "/a/b/c"),
			("./a/b/c/", "/a/b/c"),
			("/./a/b/c/", "/a/b/c"),
			("/a/./b/c/", "/a/b/c"),
			("/a/b/./c/", "/a/b/c"),
			("/a/b/c/./", "/a/b/c"),
			("././a/b/c", "/a/b/c"),
			("/././a/b/c", "/a/b/c"),
			("/a/././b/c", "/a/b/c"),
			("/a/b/././c", "/a/b/c"),
			("/a/b/c/./.", "/a/b/c"),
			("/a/b/./c///..//.././//d/../../e/", "/e"),
		];

		for case in cases {
			dbg!(case.0);

			assert_eq!(super::normalize_path(case.0), case.1);
		}
	}

	#[test]
	fn trim() {
		assert_eq!(super::trim(b"  Hello, World!"), b"Hello, World!");
		assert_eq!(super::trim(b"Hello, World!  "), b"Hello, World!");
		assert_eq!(super::trim(b"  Hello, World!  "), b"Hello, World!");
	}
}
