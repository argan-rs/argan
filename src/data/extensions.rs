use std::{any::type_name, convert::Infallible, fmt::Debug, fmt::Display, marker::PhantomData};

use argan_core::request::FromRequestRef;
use http::{Extensions, StatusCode};

use crate::{
	request::{FromRequest, Request, RequestHead},
	response::{
		BoxedErrorResponse, IntoResponse, IntoResponseHead, IntoResponseResult, Response,
		ResponseHeadParts,
	},
};

use super::*;

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

// --------------------------------------------------
// RequestExtension

pub struct RequestExtension<'r, T>(pub &'r T);

impl<'r, B, T> FromRequestRef<'r, B> for RequestExtension<'r, T>
where
	B: Sync,
	T: Clone + Send + Sync + 'static,
{
	type Error = ExtensionExtractorError<T>;

	async fn from_request_ref(request: &'r Request<B>) -> Result<Self, Self::Error> {
		request
			.extensions()
			.get::<T>()
			.map(|value| Self(value))
			.ok_or(ExtensionExtractorError(PhantomData))
	}
}

// impl<B, T> FromRequest<B> for RequestExtension<T>
// where
// 	B: Send,
// 	T: Clone + Send + Sync + 'static,
// {
// 	type Error = ExtensionExtractorError<T>;
//
// 	async fn from_request(request: Request<B>) -> Result<Self, Self::Error> {
// 		request
// 			.extensions()
// 			.get::<T>()
// 			.map(|value| Self(value.clone()))
// 			.ok_or(ExtensionExtractorError(PhantomData))
// 	}
// }

// --------------------------------------------------
// HandlerExtension

// #[derive(Clone)]
// pub struct HandlerExtension<HE>(pub HE);
//
// impl<'n, HE> FromMutRequestHead<Args<'n, HE>> for HandlerExtension<HE>
// where
// 	HE: Clone + Sync + 'static,
// {
// 	type Error = Infallible;
//
// 	#[inline]
// 	async fn from_request_head(
// 		_head: &mut RequestHead,
// 		args: &Args<'n, HE>,
// 	) -> Result<Self, Self::Error> {
// 		Ok(Self(args.handler_extension.clone().into_owned()))
// 	}
// }
//
// impl<'n, B, HE> FromRequest<B, Args<'n, HE>> for HandlerExtension<HE>
// where
// 	B: Send,
// 	HE: Clone + Send + Sync + 'static,
// {
// 	type Error = Infallible;
//
// 	#[inline]
// 	async fn from_request(_request: Request<B>, args: Args<'n, HE>) -> Result<Self, Self::Error> {
// 		Ok(Self(args.handler_extension.clone().into_owned()))
// 	}
// }

// --------------------------------------------------
// NodeExtension

// #[derive(Clone)]
// pub struct NodeExtension<NE>(pub NE);
//
// impl<'n, HE, NE> FromMutRequestHead<Args<'n, HE>> for NodeExtension<NE>
// where
// 	HE: Clone + Sync,
// 	NE: Clone + Send + Sync + 'static,
// {
// 	type Error = ExtensionExtractorError<NE>;
//
// 	#[inline]
// 	async fn from_request_head(
// 		_head: &mut RequestHead,
// 		args: &Args<'n, HE>,
// 	) -> Result<Self, Self::Error> {
// 		args
// 			.node_extensions
// 			.get_ref::<NE>()
// 			.ok_or(ExtensionExtractorError::<NE>::new())
// 			.map(|node_extension| Self(node_extension.clone()))
// 	}
// }
//
// impl<'n, B, HE, NE> FromRequest<B, Args<'n, HE>> for NodeExtension<NE>
// where
// 	B: Send,
// 	HE: Clone + Send + Sync,
// 	NE: Clone + Send + Sync + 'static,
// {
// 	type Error = ExtensionExtractorError<NE>;
//
// 	#[inline]
// 	async fn from_request(_request: Request<B>, args: Args<'n, HE>) -> Result<Self, Self::Error> {
// 		args
// 			.node_extensions
// 			.get_ref::<NE>()
// 			.ok_or(ExtensionExtractorError::<NE>::new())
// 			.map(|node_extension| Self(node_extension.clone()))
// 	}
// }

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

	#[inline(always)]
	pub fn get_ref<T: Send + Sync + 'static>(&self) -> Option<&T> {
		self.0.get::<T>()
	}

	#[inline(always)]
	pub(crate) fn to_owned(self) -> NodeExtensions<'static> {
		NodeExtensions(Cow::Owned(self.0.into_owned()))
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
	fn into_response_head(
		self,
		mut head: ResponseHeadParts,
	) -> Result<ResponseHeadParts, BoxedErrorResponse> {
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
