use std::{any::type_name, convert::Infallible, fmt::Debug, fmt::Display, marker::PhantomData};

use http::StatusCode;

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

impl<Args, Ext, T> FromRequestHead<Args, Ext> for RequestExtension<T>
where
	Args: for<'n> Arguments<'n, Ext> + Send,
	Ext: Sync,
	T: Clone + Send + Sync + 'static,
{
	type Error = ExtensionExtractorError<T>;

	async fn from_request_head(
		head: &mut RequestHead,
		_args: &mut Args,
	) -> Result<Self, Self::Error> {
		head
			.extensions
			.get::<T>()
			.map(|value| Self(value.clone()))
			.ok_or(ExtensionExtractorError(PhantomData))
	}
}

impl<B, Args, Ext, T> FromRequest<B, Args, Ext> for RequestExtension<T>
where
	B: Send,
	Args: for<'n> Arguments<'n, Ext> + Send,
	Ext: Sync,
	T: Clone + Send + Sync + 'static,
{
	type Error = ExtensionExtractorError<T>;

	async fn from_request(request: Request<B>, _args: &mut Args) -> Result<Self, Self::Error> {
		let (mut head, _) = request.into_parts();

		Self::from_request_head(&mut head, _args).await
	}
}

// --------------------------------------------------
// HandlerExtension

#[derive(Clone)]
pub struct HandlerExtension<Ext>(pub Ext);

impl<Args, Ext> FromRequestHead<Args, Ext> for HandlerExtension<Ext>
where
	Args: for<'n> Arguments<'n, Ext> + Send,
	Ext: Clone + Sync + 'static,
{
	type Error = Infallible;

	#[inline]
	async fn from_request_head(
		_head: &mut RequestHead,
		args: &mut Args,
	) -> Result<Self, Self::Error> {
		Ok(HandlerExtension(args.handler_extension().clone()))
	}
}

impl<B, Args, Ext> FromRequest<B, Args, Ext> for HandlerExtension<Ext>
where
	B: Send,
	Args: for<'n> Arguments<'n, Ext> + Send,
	Ext: Clone + Sync + 'static,
{
	type Error = Infallible;

	#[inline]
	async fn from_request(_request: Request<B>, args: &mut Args) -> Result<Self, Self::Error> {
		Ok(HandlerExtension(args.handler_extension().clone()))
	}
}

// --------------------------------------------------
// NodeExtension

#[derive(Clone)]
pub struct NodeExtension<Ext>(pub Ext);

impl<Args, Ext> FromRequestHead<Args, Ext> for NodeExtension<Ext>
where
	Args: for<'n> Arguments<'n, Ext> + Send,
	Ext: Clone + Send + Sync + 'static,
{
	type Error = ExtensionExtractorError<Ext>;

	#[inline]
	async fn from_request_head(
		_head: &mut RequestHead,
		args: &mut Args,
	) -> Result<Self, Self::Error> {
		args
			.node_extension::<Ext>()
			.ok_or(ExtensionExtractorError::<Ext>::new())
			.map(|node_extension| NodeExtension(node_extension.clone()))
	}
}

impl<B, Args, Ext> FromRequest<B, Args, Ext> for NodeExtension<Ext>
where
	B: Send,
	Args: for<'n> Arguments<'n, Ext> + Send,
	Ext: Clone + Send + Sync + 'static,
{
	type Error = ExtensionExtractorError<Ext>;

	#[inline]
	async fn from_request(_request: Request<B>, args: &mut Args) -> Result<Self, Self::Error> {
		args
			.node_extension::<Ext>()
			.ok_or(ExtensionExtractorError::<Ext>::new())
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
