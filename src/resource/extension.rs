use std::borrow::Cow;

use http::Extensions;

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

#[derive(Clone)]
pub struct ResourceExtensions<'r>(Cow<'r, Extensions>);

impl<'r> ResourceExtensions<'r> {
	#[inline(always)]
	pub(crate) fn new(extensions: &'r Extensions) -> Self {
		Self(Cow::Borrowed(extensions))
	}

	#[inline(always)]
	pub fn get_ref<T: Send + Sync + 'static>(&self) -> Option<&T> {
		self.0.get::<T>()
	}

	#[inline(always)]
	pub fn is_empty(&self) -> bool {
		self.0.is_empty()
	}

	#[inline(always)]
	pub fn len(&self) -> usize {
		self.0.len()
	}

	#[inline(always)]
	pub(crate) fn into_owned(self) -> ResourceExtensions<'static> {
		ResourceExtensions(Cow::<'static, _>::Owned(self.0.into_owned()))
	}
}
