use std::{borrow::Cow, convert::Infallible};

use http::{Extensions, StatusCode};

use crate::{
	handler::Args,
	request::{FromRequest, FromRequestHead, Request, RequestHead},
};

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

#[derive(Default, Clone)]
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

// --------------------------------------------------
// ResourceExtension

pub struct ResourceExtension<RE>(RE);

impl<HE, RE> FromRequestHead<HE> for ResourceExtension<RE>
where
	HE: Sync,
	RE: Clone + Send + Sync + 'static,
{
	type Error = StatusCode; // ???

	#[inline]
	async fn from_request_head(
		head: &mut RequestHead,
		args: &mut Args<'_, HE>,
	) -> Result<Self, Self::Error> {
		match args.resource_extensions.get_ref::<RE>() {
			Some(value) => Ok(ResourceExtension(value.clone())),
			None => Err(StatusCode::INTERNAL_SERVER_ERROR),
		}
	}
}

impl<B, HE, RE> FromRequest<B, HE> for ResourceExtension<RE>
where
	B: Send,
	HE: Sync,
	RE: Clone + Send + Sync + 'static,
{
	type Error = StatusCode; // ???

	#[inline]
	async fn from_request(request: Request<B>, args: &mut Args<'_, HE>) -> Result<Self, Self::Error> {
		let (mut head, _) = request.into_parts();

		<ResourceExtension<RE> as FromRequestHead<HE>>::from_request_head(&mut head, args).await
	}
}

// --------------------------------------------------------------------------------
