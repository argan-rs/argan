use std::borrow::Cow;

use http::Extensions;

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

#[derive(Clone)]
pub struct ResourceExtensions<'r>(Cow<'r, Extensions>);

impl<'r> ResourceExtensions<'r> {
	#[inline(always)]
	pub(crate) fn new_borrowed(extensions: &'r Extensions) -> Self {
		Self(Cow::Borrowed(extensions))
	}

	#[inline(always)]
	pub(crate) fn new_owned(extensions: Extensions) -> ResourceExtensions<'static> {
		ResourceExtensions(Cow::Owned(extensions))
	}

	#[inline(always)]
	pub fn get_ref<T: Send + Sync + 'static>(&self) -> Option<&T> {
		self.0.get::<T>()
	}

	#[inline(always)]
	pub(crate) fn into_owned(self) -> ResourceExtensions<'static> {
		ResourceExtensions(Cow::<'static, _>::Owned(self.0.into_owned()))
	}
}

impl AsRef<Extensions> for ResourceExtensions<'_> {
	#[inline(always)]
	fn as_ref(&self) -> &Extensions {
		self.0.as_ref()
	}
}
