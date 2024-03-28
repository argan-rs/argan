use std::{any::type_name, convert::Infallible, fmt::Debug, fmt::Display, marker::PhantomData};

use http::{Extensions, StatusCode};

use crate::{
	request::{FromRequest, FromRequestHead, Request, RequestHead},
	response::{
		BoxedErrorResponse, IntoResponse, IntoResponseHead, IntoResponseResult, Response, ResponseHead,
	},
};

use super::*;

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

// --------------------------------------------------
// RequestExtension

pub struct RequestExtension<T>(pub T);

impl<'n, HE, T> FromRequestHead<Args<'n, HE>> for RequestExtension<T>
where
	HE: Sync,
	T: Clone + Send + Sync + 'static,
{
	type Error = ExtensionExtractorError<T>;

	async fn from_request_head(
		head: &mut RequestHead,
		_args: &mut Args<'n, HE>,
	) -> Result<Self, Self::Error> {
		head
			.extensions
			.get::<T>()
			.map(|value| Self(value.clone()))
			.ok_or(ExtensionExtractorError(PhantomData))
	}
}

impl<'n, B, HE, T> FromRequest<B, Args<'n, HE>> for RequestExtension<T>
where
	B: Send,
	HE: Sync,
	T: Clone + Send + Sync + 'static,
{
	type Error = ExtensionExtractorError<T>;

	async fn from_request(
		request: Request<B>,
		_args: &mut Args<'n, HE>,
	) -> Result<Self, Self::Error> {
		let (mut head, _) = request.into_parts();

		Self::from_request_head(&mut head, _args).await
	}
}

// --------------------------------------------------
// HandlerExtension

#[derive(Clone)]
pub struct HandlerExtension<HE>(pub HE);

impl<'n, HE> FromRequestHead<Args<'n, HE>> for HandlerExtension<HE>
where
	HE: Clone + Sync + 'static,
{
	type Error = Infallible;

	#[inline]
	async fn from_request_head(
		_head: &mut RequestHead,
		args: &mut Args<'n, HE>,
	) -> Result<Self, Self::Error> {
		Ok(Self(args.handler_extension.clone()))
	}
}

impl<'n, B, HE> FromRequest<B, Args<'n, HE>> for HandlerExtension<HE>
where
	B: Send,
	HE: Clone + Sync + 'static,
{
	type Error = Infallible;

	#[inline]
	async fn from_request(
		_request: Request<B>,
		args: &mut Args<'n, HE>,
	) -> Result<Self, Self::Error> {
		Ok(HandlerExtension(args.handler_extension.clone()))
	}
}

// --------------------------------------------------
// NodeExtension

#[derive(Clone)]
pub struct NodeExtension<NE>(pub NE);

impl<'n, HE, NE> FromRequestHead<Args<'n, HE>> for NodeExtension<NE>
where
	HE: Sync,
	NE: Clone + Send + Sync + 'static,
{
	type Error = ExtensionExtractorError<NE>;

	#[inline]
	async fn from_request_head(
		_head: &mut RequestHead,
		args: &mut Args<'n, HE>,
	) -> Result<Self, Self::Error> {
		args
			.node_extensions
			.get_ref::<NE>()
			.ok_or(ExtensionExtractorError::<NE>::new())
			.map(|node_extension| NodeExtension(node_extension.clone()))
	}
}

impl<'n, B, HE, NE> FromRequest<B, Args<'n, HE>> for NodeExtension<NE>
where
	B: Send,
	HE: Sync,
	NE: Clone + Send + Sync + 'static,
{
	type Error = ExtensionExtractorError<NE>;

	#[inline]
	async fn from_request(
		_request: Request<B>,
		args: &mut Args<'n, HE>,
	) -> Result<Self, Self::Error> {
		args
			.node_extensions
			.get_ref::<NE>()
			.ok_or(ExtensionExtractorError::<NE>::new())
			.map(|node_extension| NodeExtension(node_extension.clone()))
	}
}

// -------------------------
// ExtensionExtractorError

pub struct ExtensionExtractorError<T>(PhantomData<T>);

impl<T> ExtensionExtractorError<T> {
	fn new() -> Self {
		Self(PhantomData)
	}
}

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
// NodeExtensions

#[derive(Clone)]
pub struct NodeExtensions<'n>(NodeExtensionsValue<'n>);

#[derive(Clone)]
pub enum NodeExtensionsValue<'n> {
	Borrowed(&'n Extensions),
	Owned(Extensions),
}

impl<'n> NodeExtensions<'n> {
	#[inline(always)]
	pub(crate) fn new_borrowed(extensions: &'n Extensions) -> Self {
		Self(NodeExtensionsValue::Borrowed(extensions))
	}

	#[inline(always)]
	pub(crate) fn new_owned(extensions: Extensions) -> Self {
		Self(NodeExtensionsValue::Owned(extensions))
	}

	#[inline(always)]
	pub fn get_ref<T: Send + Sync + 'static>(&self) -> Option<&T> {
		match self.0 {
			NodeExtensionsValue::Borrowed(extensions) => extensions.get::<T>(),
			NodeExtensionsValue::Owned(ref extensions) => extensions.get::<T>(),
		}
	}

	#[inline(always)]
	pub(crate) fn into_owned(self) -> NodeExtensions<'static> {
		match self.0 {
			NodeExtensionsValue::Borrowed(extensions) => {
				NodeExtensions(NodeExtensionsValue::Owned(extensions.clone()))
			}
			NodeExtensionsValue::Owned(extensions) => {
				NodeExtensions(NodeExtensionsValue::Owned(extensions))
			}
		}
	}
}

// --------------------------------------------------------------------------------
// ResponseExtension

pub struct ResponseExtension<T>(pub T);

impl<T> IntoResponseHead for ResponseExtension<T>
where
	T: Clone + Send + Sync + 'static,
{
	#[inline]
	fn into_response_head(self, mut head: ResponseHead) -> Result<ResponseHead, BoxedErrorResponse> {
		let ResponseExtension(value) = self;

		if head.extensions.insert(value).is_some() {
			return Err(ResponseExtensionError::<T>(PhantomData).into());
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

// -------------------------
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
