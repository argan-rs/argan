use std::{
	convert::Infallible,
	fmt::{Debug, Display},
	future::{ready, Future, Ready},
};

use argan_core::{body::Body, BoxedFuture};
use futures_util::TryFutureExt;
use http::{
	header::{ToStrError, CONTENT_TYPE},
	request::{self, Parts},
	HeaderName, StatusCode,
};
use serde::{
	de::{DeserializeOwned, Error},
	Deserialize, Deserializer,
};

use crate::{
	common::{marker::Sealed, Uncloneable},
	data::header::ContentTypeError,
	handler::Args,
	pattern,
	response::{BoxedErrorResponse, IntoResponse, IntoResponseHead, Response},
	routing::RoutingState,
	ImplError, StdError,
};

// ----------

pub use argan_core::request::*;

// --------------------------------------------------

pub mod websocket;

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

// --------------------------------------------------
// RequestExt

pub trait Extract<B>: Sized + Sealed {
	fn extract_ref<'r, T>(&'r self) -> Result<T, T::Error>
	where
		T: FromRequestRef<'r, B, Args<'static, ()>>;

	fn extract_ref_with_args<'r, Ext: Clone + 'static, T>(
		&'r self,
		args: &'r Args<'static, Ext>,
	) -> Result<T, T::Error>
	where
		T: FromRequestRef<'r, B, Args<'static, Ext>>;

	fn extract<T>(self) -> impl Future<Output = Result<T, T::Error>> + Send
	where
		T: FromRequest<B, Args<'static, ()>>;

	fn extract_with_args<Ext: Clone + 'static, T>(
		self,
		args: Args<'static, Ext>,
	) -> impl Future<Output = Result<T, T::Error>> + Send
	where
		T: FromRequest<B, Args<'static, Ext>>;
}

impl<B> Extract<B> for Request<B> {
	fn extract_ref<'r, T>(&'r self) -> Result<T, T::Error>
	where
		T: FromRequestRef<'r, B, Args<'static, ()>>,
	{
		T::from_request_ref(self, None)
	}

	fn extract_ref_with_args<'r, Ext: Clone + 'static, T>(
		&'r self,
		args: &'r Args<'static, Ext>,
	) -> Result<T, T::Error>
	where
		T: FromRequestRef<'r, B, Args<'static, Ext>>,
	{
		T::from_request_ref(self, Some(args))
	}

	fn extract<T>(self) -> impl Future<Output = Result<T, T::Error>> + Send
	where
		T: FromRequest<B, Args<'static, ()>>,
	{
		T::from_request(self, Args::new())
	}

	fn extract_with_args<Ext: Clone + 'static, T>(
		self,
		args: Args<'static, Ext>,
	) -> impl Future<Output = Result<T, T::Error>> + Send
	where
		T: FromRequest<B, Args<'static, Ext>>,
	{
		T::from_request(self, args)
	}
}

impl<B> Sealed for Request<B> {}

// --------------------------------------------------
// PathParams

pub struct PathParams<T>(pub T);

impl<'de, T> PathParams<T>
where
	T: Deserialize<'de>,
{
	pub fn deserialize<D: Deserializer<'de>>(&mut self, deserializer: D) -> Result<(), D::Error> {
		self.0 = T::deserialize(deserializer)?;

		Ok(())
	}
}

impl<'r, B, HE, T> FromRequestRef<'r, B, Args<'static, HE>> for PathParams<T>
where
	HE: Clone,
	T: Deserialize<'r> + 'r,
{
	type Error = PathParamsError;

	fn from_request_ref(
		request: &'r Request<B>,
		some_args: Option<&'r Args<'static, HE>>,
	) -> Result<Self, Self::Error> {
		let args = some_args.unwrap(); // TODO

		let mut from_params_list = args.routing_state.uri_params.deserializer();

		T::deserialize(&mut from_params_list)
			.map(|value| Self(value))
			.map_err(Into::into)
	}
}

impl<'n, HE, T> FromRequestHead<Args<'n, HE>> for PathParams<T>
where
	HE: Clone + Sync,
	T: DeserializeOwned,
{
	type Error = PathParamsError;

	async fn from_request_head(
		head: &mut RequestHead,
		args: &Args<'n, HE>,
	) -> Result<Self, Self::Error> {
		let mut from_params_list = args.routing_state.uri_params.deserializer();

		T::deserialize(&mut from_params_list)
			.map(|value| Self(value))
			.map_err(Into::into)
	}
}

impl<'n, B, HE, T> FromRequest<B, Args<'n, HE>> for PathParams<T>
where
	B: Send,
	HE: Clone + Send + Sync,
	T: DeserializeOwned,
{
	type Error = PathParamsError;

	async fn from_request(request: Request<B>, args: Args<'n, HE>) -> Result<Self, Self::Error> {
		let (mut head, _) = request.into_parts();

		Self::from_request_head(&mut head, &args).await
	}
}

impl<T> Debug for PathParams<T>
where
	T: Debug,
{
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_tuple("PathParams").field(&self.0).finish()
	}
}

// ----------

#[derive(Debug, crate::ImplError)]
#[error(transparent)]
pub struct PathParamsError(#[from] pub(crate) pattern::DeserializerError);

impl IntoResponse for PathParamsError {
	fn into_response(self) -> Response {
		match self.0 {
			pattern::DeserializerError::ParsingFailue(_) => StatusCode::NOT_FOUND.into_response(),
			_ => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
		}
	}
}

// --------------------------------------------------
// QueryParams

pub struct QueryParams<T>(pub T);

impl<'n, HE, T> FromRequestHead<Args<'n, HE>> for QueryParams<T>
where
	HE: Clone + Sync,
	T: DeserializeOwned,
{
	type Error = QueryParamsError;

	async fn from_request_head(
		head: &mut RequestHead,
		_args: &Args<'n, HE>,
	) -> Result<Self, Self::Error> {
		let query_string = head
			.uri
			.query()
			.ok_or(QueryParamsError(QueryParamsErrorValue::NoDataIsAvailable))?;

		serde_urlencoded::from_str::<T>(query_string)
			.map(|value| Self(value))
			.map_err(|error| QueryParamsError(error.into()))
	}
}

impl<'n, B, HE, T> FromRequest<B, Args<'n, HE>> for QueryParams<T>
where
	B: Send,
	HE: Clone + Send + Sync,
	T: DeserializeOwned,
{
	type Error = QueryParamsError;

	async fn from_request(request: Request<B>, _args: Args<'n, HE>) -> Result<Self, Self::Error> {
		let (mut head, _) = request.into_parts();

		Self::from_request_head(&mut head, &_args).await
	}
}

#[derive(Debug, crate::ImplError)]
#[error(transparent)]
pub struct QueryParamsError(#[from] QueryParamsErrorValue);

impl IntoResponse for QueryParamsError {
	fn into_response(self) -> Response {
		StatusCode::BAD_REQUEST.into_response()
	}
}

#[derive(Debug, crate::ImplError)]
enum QueryParamsErrorValue {
	#[error("no data is available")]
	NoDataIsAvailable,
	#[error(transparent)]
	InvalidData(#[from] serde_urlencoded::de::Error),
}

// --------------------------------------------------
// Remaining path ref

pub enum RemainingPathRef<'r> {
	Value(&'r str),
	None,
}

impl<'r, B, HE: Clone> FromRequestRef<'r, B, Args<'static, HE>> for RemainingPathRef<'r> {
	type Error = Infallible;

	fn from_request_ref(
		request: &'r Request<B>,
		some_args: Option<&'r Args<'static, HE>>,
	) -> Result<Self, Self::Error> {
		some_args.map_or(Ok(RemainingPathRef::None), |args| {
			args
				.routing_state
				.route_traversal
				.remaining_segments(request.uri().path())
				.map_or(Ok(RemainingPathRef::None), |remaining_path| {
					Ok(RemainingPathRef::Value(remaining_path))
				})
		})
	}
}

// --------------------------------------------------
// Remaining path

pub enum RemainingPath {
	Value(Box<str>),
	None,
}

impl<'n, HE> FromRequestHead<Args<'n, HE>> for RemainingPath
where
	HE: Clone + Sync,
{
	type Error = Infallible;

	async fn from_request_head(
		head: &mut RequestHead,
		args: &Args<'n, HE>,
	) -> Result<Self, Self::Error> {
		args
			.routing_state
			.route_traversal
			.remaining_segments(head.uri.path())
			.map_or(Ok(RemainingPath::None), |remaining_path| {
				Ok(RemainingPath::Value(remaining_path.into()))
			})
	}
}

impl<'n, B, HE> FromRequest<B, Args<'n, HE>> for RemainingPath
where
	B: Send,
	HE: Clone + Send + Sync,
{
	type Error = Infallible;

	async fn from_request(request: Request<B>, args: Args<'n, HE>) -> Result<Self, Self::Error> {
		args
			.routing_state
			.route_traversal
			.remaining_segments(request.uri().path())
			.map_or(Ok(RemainingPath::None), |remaining_path| {
				Ok(RemainingPath::Value(remaining_path.into()))
			})
	}
}

// --------------------------------------------------------------------------------
