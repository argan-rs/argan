use std::{any::type_name, fmt::Display};

use http::Extensions;

use crate::response::{BoxedErrorResponse, IntoResponseResult};

use super::*;

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

// --------------------------------------------------
// RequestExtension

pub struct RequestExtension<T>(pub T);

impl<E, T> FromRequestHead<E> for RequestExtension<T>
where
	E: Sync,
	T: Clone + Send + Sync + 'static,
{
	type Error = ExtensionExtractorError<T>;

	async fn from_request_head(
		head: &mut RequestHead,
		_args: &mut Args<'_, E>,
	) -> Result<Self, Self::Error> {
		head
			.extensions
			.get::<T>()
			.map(|value| Self(value.clone()))
			.ok_or(ExtensionExtractorError(PhantomData))
	}
}

impl<B, E, T> FromRequest<B, E> for RequestExtension<T>
where
	B: Send,
	E: Sync,
	T: Clone + Send + Sync + 'static,
{
	type Error = ExtensionExtractorError<T>;

	async fn from_request(request: Request<B>, _args: &mut Args<'_, E>) -> Result<Self, Self::Error> {
		let (mut head, _) = request.into_parts();

		Self::from_request_head(&mut head, _args).await
	}
}

// --------------------------------------------------
// ResponseExtension

pub struct ResponseExtension<T>(pub T);

impl<T> IntoResponseHead for ResponseExtension<T>
where
	T: Clone + Send + Sync + 'static,
{
	type Error = ResponseExtensionError<T>;

	#[inline]
	fn into_response_head(self, mut head: ResponseHead) -> Result<ResponseHead, Self::Error> {
		let ResponseExtension(value) = self;

		if head.extensions.insert(value).is_some() {
			return Err(ResponseExtensionError(PhantomData));
		}

		Ok(head)
	}
}

impl<T> IntoResponseResult for ResponseExtension<T>
where
	T: Clone + Send + Sync + 'static,
{
	#[inline]
	fn into_response_result(self) -> Result<Response, BoxedErrorResponse> {
		let ResponseExtension(value) = self;

		let mut response = Response::default();
		if response.extensions_mut().insert(value).is_some() {
			return Err(ResponseExtensionError::<T>(PhantomData).into());
		}

		Ok(response)
	}
}

// --------------------------------------------------
// HandlerExtension

#[derive(Clone)]
pub struct HandlerExtension<E>(E);

impl<E> FromRequestHead<E> for HandlerExtension<E>
where
	E: Clone + Sync,
{
	type Error = Infallible;

	#[inline]
	async fn from_request_head(
		head: &mut RequestHead,
		args: &mut Args<'_, E>,
	) -> Result<Self, Self::Error> {
		Ok(HandlerExtension(args.handler_extension.clone()))
	}
}

impl<B, E> FromRequest<B, E> for HandlerExtension<E>
where
	B: Send,
	E: Clone + Sync,
{
	type Error = Infallible;

	#[inline]
	async fn from_request(request: Request<B>, args: &mut Args<'_, E>) -> Result<Self, Self::Error> {
		Ok(HandlerExtension(args.handler_extension.clone()))
	}
}

// --------------------------------------------------
// NodeExtension

pub struct NodeExtension<NE>(NE);

impl<HE, NE> FromRequestHead<HE> for NodeExtension<NE>
where
	HE: Sync,
	NE: Clone + Send + Sync + 'static,
{
	type Error = ExtensionExtractorError<NE>;

	#[inline]
	async fn from_request_head(
		head: &mut RequestHead,
		args: &mut Args<'_, HE>,
	) -> Result<Self, Self::Error> {
		args
			.node_extensions
			.get_ref::<NE>()
			.map(|value| Self(value.clone()))
			.ok_or(ExtensionExtractorError(PhantomData))
	}
}

impl<B, HE, NE> FromRequest<B, HE> for NodeExtension<NE>
where
	B: Send,
	HE: Sync,
	NE: Clone + Send + Sync + 'static,
{
	type Error = ExtensionExtractorError<NE>;

	#[inline]
	async fn from_request(request: Request<B>, args: &mut Args<'_, HE>) -> Result<Self, Self::Error> {
		args
			.node_extensions
			.get_ref::<NE>()
			.map(|value| Self(value.clone()))
			.ok_or(ExtensionExtractorError(PhantomData))
	}
}

// --------------------------------------------------
// ResourceExtensions

#[derive(Clone)]
pub struct NodeExtensions<'r>(Cow<'r, Extensions>);

impl<'r> NodeExtensions<'r> {
	#[inline(always)]
	pub(crate) fn new_borrowed(extensions: &'r Extensions) -> Self {
		Self(Cow::Borrowed(extensions))
	}

	#[inline(always)]
	pub(crate) fn new_owned(extensions: Extensions) -> NodeExtensions<'static> {
		NodeExtensions(Cow::Owned(extensions))
	}

	#[inline(always)]
	pub fn get_ref<T: Send + Sync + 'static>(&self) -> Option<&T> {
		self.0.get::<T>()
	}

	#[inline(always)]
	pub(crate) fn take(&mut self) -> NodeExtensions<'_> {
		NodeExtensions(std::mem::take(&mut self.0))
	}

	#[inline(always)]
	pub(crate) fn into_owned(self) -> NodeExtensions<'static> {
		NodeExtensions(Cow::<'static, _>::Owned(self.0.into_owned()))
	}
}

// --------------------------------------------------
// ExtensionExtractorError

pub struct ExtensionExtractorError<T>(PhantomData<T>);

impl<T> Debug for ExtensionExtractorError<T> {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "ExtensionExtractorError<{}>", type_name::<T>())
	}
}

impl<T> Display for ExtensionExtractorError<T> {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "extension {0} doesn't exist", type_name::<T>())
	}
}

impl<T> crate::StdError for ExtensionExtractorError<T> {}

impl<T> IntoResponse for ExtensionExtractorError<T> {
	fn into_response(self) -> Response {
		StatusCode::INTERNAL_SERVER_ERROR.into_response()
	}
}

// --------------------------------------------------
// ResponseExtensionError

pub struct ResponseExtensionError<T>(PhantomData<T>);

impl<T> Debug for ResponseExtensionError<T> {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "ResponseExtensionError<{}>", type_name::<T>())
	}
}

impl<T> Display for ResponseExtensionError<T> {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(
			f,
			"type {0} has already been used as a response extension",
			type_name::<T>()
		)
	}
}

impl<T> crate::StdError for ResponseExtensionError<T> {}

impl<T> IntoResponse for ResponseExtensionError<T> {
	fn into_response(self) -> Response {
		StatusCode::INTERNAL_SERVER_ERROR.into_response()
	}
}

// --------------------------------------------------------------------------------
