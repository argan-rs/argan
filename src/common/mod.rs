use std::{
	any::Any,
	borrow::Cow,
	fmt::Display,
	future::Future,
	io::ErrorKind,
	pin::Pin,
	task::{Context, Poll},
	time::Duration,
};

use http::{Extensions, StatusCode};

use crate::{
	handler::BoxedHandler,
	pattern::Pattern,
	response::{ErrorResponse, IntoResponse, Response},
	routing::RouteSegments,
};

// --------------------------------------------------

#[macro_use]
pub(crate) mod macros;

pub mod config;
pub use config::_with_request_extensions_modifier;

pub(crate) mod timer;

#[cfg(test)]
pub(crate) mod test_helpers;

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

// --------------------------------------------------
// Markers

pub(crate) mod marker {
	pub trait Sealed {}

	// ----------

	pub struct Private;
}

// Used when expecting a valid value in Options or Results.
pub(crate) const SCOPE_VALIDITY: &'static str = "scope validity";

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

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------
// Temp Place

// #[inline]
// pub(crate) fn node_extensions_replaced<'e>(
// 	&mut self,
// 	extensions: &'e Extensions,
// ) -> Args<'e, PrivateExt> {
// 	let Args {
// 		private_extension,
// 		node_extension: node_extensions,
// 		handler_extension,
// 	} = self;
//
// 	let mut args = Args {
// 		private_extension: std::mem::take(private_extension),
// 		node_extension: NodeExtensions::new_borrowed(extensions),
// 		handler_extension: &(),
// 	};
//
// 	args
// }

// --------------------------------------------------
// NodeExtension

// pub struct NodeExtension<NE>(NE);
//
// impl<HE, NE> FromRequestHead<HE> for NodeExtension<NE>
// where
// 	HE: Sync,
// 	NE: Clone + Send + Sync + 'static,
// {
// 	type Error = ExtensionExtractorError<NE>;
//
// 	#[inline]
// 	async fn from_request_head(
// 		head: &mut RequestHead,
// 		args: &mut impl Args<'_, HE>,
// 	) -> Result<Self, Self::Error> {
// 		args
// 			.node_extension()
// 			.get_ref::<NE>()
// 			.map(|value| Self(value.clone()))
// 			.ok_or(ExtensionExtractorError(PhantomData))
// 	}
// }
//
// impl<B, HE, NE> FromRequest<B, HE> for NodeExtension<NE>
// where
// 	B: Send,
// 	HE: Sync,
// 	NE: Clone + Send + Sync + 'static,
// {
// 	type Error = ExtensionExtractorError<NE>;
//
// 	#[inline]
// 	async fn from_request(request: Request<B>, args: &mut impl Args<'_, HE>) -> Result<Self, Self::Error> {
// 		args
// 			.node_extension()
// 			.get_ref::<NE>()
// 			.map(|value| Self(value.clone()))
// 			.ok_or(ExtensionExtractorError(PhantomData))
// 	}
// }

// --------------------------------------------------
// NodeExtensions

// #[derive(Clone)]
// pub struct NodeExtensions<'r>(Cow<'r, Extensions>);
//
// impl<'r> NodeExtensions<'r> {
// 	#[inline(always)]
// 	pub(crate) fn new_borrowed(extensions: &'r Extensions) -> Self {
// 		Self(Cow::Borrowed(extensions))
// 	}
//
// 	#[inline(always)]
// 	pub(crate) fn new_owned(extensions: Extensions) -> NodeExtensions<'static> {
// 		NodeExtensions(Cow::Owned(extensions))
// 	}
//
// 	#[inline(always)]
// 	pub fn get_ref<T: Send + Sync + 'static>(&self) -> Option<&T> {
// 		self.0.get::<T>()
// 	}
//
// 	#[inline(always)]
// 	pub(crate) fn take(&mut self) -> NodeExtensions<'_> {
// 		NodeExtensions(std::mem::take(&mut self.0))
// 	}
//
// 	#[inline(always)]
// 	pub(crate) fn into_owned(self) -> NodeExtensions<'static> {
// 		NodeExtensions(Cow::<'static, _>::Owned(self.0.into_owned()))
// 	}
// }

// --------------------------------------------------
// BoxedAny

pub(crate) struct BoxedAny(Box<dyn AnyCloneable + Send + Sync>);

impl BoxedAny {
	pub(crate) fn new<T: Any + Clone + Send + Sync>(value: T) -> Self {
		Self(Box::new(value))
	}

	pub(crate) fn as_any(&self) -> &(dyn Any + Send + Sync) {
		self.0.as_ref().as_any()
	}
}

impl Clone for BoxedAny {
	fn clone(&self) -> Self {
		self.0.as_ref().boxed_clone()
	}
}

// --------------------------------------------------
// AnyClonealbe

trait AnyCloneable {
	fn as_any(&self) -> &(dyn Any + Send + Sync);
	fn boxed_clone(&self) -> BoxedAny;
}

impl<A: Any + Clone + Send + Sync> AnyCloneable for A {
	fn as_any(&self) -> &(dyn Any + Send + Sync) {
		self
	}

	fn boxed_clone(&self) -> BoxedAny {
		BoxedAny::new(self.clone())
	}
}

// --------------------------------------------------
// NodeExtensions

// pub(crate) enum OptionCow<'a, T> {
// 	None,
// 	Borrowed(&'a T),
// 	Owned(T),
// }
//
// impl<T: Clone + 'static> Clone for OptionCow<'_, T> {
// 	fn clone(&self) -> OptionCow<'static, T> {
// 		match self {
// 			Self::None => Self::None,
// 			Self::Borrowed(value) => Self::Owned((*value).clone()),
// 			Self::Owned(value) => Self::Owned(value.clone()),
// 		}
// 	}
// }

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

#[cfg(test)]
mod test {
	use super::*;

	// --------------------------------------------------------------------------------
	// --------------------------------------------------------------------------------

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
