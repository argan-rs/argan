//! Common types and functions.

// ----------

use std::{
	any::{Any, TypeId},
	fmt::{self, Debug},
	net::SocketAddr,
};

use crate::{handler::BoxedHandler, pattern::Pattern, request::routing::RouteSegments};

// ----------

pub use argan_core::{BoxedError, BoxedFuture};

// --------------------------------------------------

#[macro_use]
pub(crate) mod macros;

pub mod node_properties;

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
// NodeExtension

/// An Extension of a node ([`Router`](crate::Router), [`Resource`](crate::Resource)).
///
/// Available in the [`Args`](crate::handler::Args) while the request is being routed.
pub struct NodeExtension(Option<Box<dyn AnyCloneable + Send + Sync>>);

impl NodeExtension {
	#[inline(always)]
	pub(crate) fn new() -> Self {
		NodeExtension(None)
	}

	#[inline(always)]
	pub(crate) fn set_value_to<E: Clone + Send + Sync + 'static>(&mut self, extension: E) {
		self.0 = Some(Box::new(extension));
	}

	#[inline(always)]
	pub(crate) fn has_value(&self) -> bool {
		self.0.is_some()
	}

	/// Attempts to downcast to a concrete type `E`.
	#[inline(always)]
	pub fn downcast_to<E: Any>(self) -> Result<Box<E>, Self> {
		let NodeExtension(Some(boxed_any_cloneable)) = self else {
			return Err(Self(None));
		};

		if boxed_any_cloneable.as_ref().concrete_type_id() == TypeId::of::<E>() {
			Ok(
				boxed_any_cloneable
					.into_boxed_any()
					.downcast()
					.expect(SCOPE_VALIDITY),
			)
		} else {
			Err(Self(Some(boxed_any_cloneable)))
		}
	}

	/// Returns some reference to the concrete type `E` if the inner value is of that type
	/// or returns `None`.
	#[inline(always)]
	pub fn downcast_to_ref<E: Any>(&self) -> Option<&E> {
		self
			.0
			.as_ref()
			.and_then(|boxed_any_cloneable| boxed_any_cloneable.as_ref().as_any_ref().downcast_ref())
	}

	/// Returns some mutable reference to the concrete type `E` if the inner value is of
	/// that type or returns `None`.
	#[inline(always)]
	pub fn downcast_to_mut<E: Any>(&mut self) -> Option<&mut E> {
		self
			.0
			.as_mut()
			.and_then(|boxed_any_cloneable| boxed_any_cloneable.as_mut().as_any_mut().downcast_mut())
	}
}

impl Debug for NodeExtension {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.debug_struct("NodeExtension").finish_non_exhaustive()
	}
}

impl Clone for NodeExtension {
	fn clone(&self) -> Self {
		Self(
			self
				.0
				.as_ref()
				.map(|boxed_any_cloneable| boxed_any_cloneable.as_ref().boxed_clone()),
		)
	}
}

// --------------------------------------------------
// AnyCloneable

trait AnyCloneable: 'static {
	fn concrete_type_id(&self) -> TypeId {
		TypeId::of::<Self>()
	}

	fn as_any_ref(&self) -> &dyn Any;
	fn as_any_mut(&mut self) -> &mut dyn Any;
	fn into_boxed_any(self: Box<Self>) -> Box<dyn Any>;

	fn boxed_clone(&self) -> Box<dyn AnyCloneable + Send + Sync>;
}

impl<T> AnyCloneable for T
where
	T: Clone + Send + Sync + 'static,
{
	fn as_any_ref(&self) -> &dyn Any {
		self
	}

	fn as_any_mut(&mut self) -> &mut dyn Any {
		self
	}

	fn into_boxed_any(self: Box<Self>) -> Box<dyn Any> {
		self
	}

	fn boxed_clone(&self) -> Box<dyn AnyCloneable + Send + Sync> {
		Box::new(self.clone())
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
	use super::*;

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

	#[test]
	fn node_extension() {
		#[derive(Debug, Clone)]
		struct A;

		#[derive(Debug, Clone)]
		struct B;

		// ----------

		let boxed_any_cloneable = Box::new(A) as Box<dyn AnyCloneable + Send + Sync>;
		assert_eq!(boxed_any_cloneable.concrete_type_id(), TypeId::of::<A>());

		let mut node_extension = NodeExtension(Some(boxed_any_cloneable));

		assert!(node_extension.downcast_to_ref::<A>().is_some());
		assert!(node_extension.downcast_to_mut::<A>().is_some());

		let node_extension = node_extension.downcast_to::<B>().unwrap_err();

		assert!(node_extension.downcast_to::<A>().is_ok())
	}
}
