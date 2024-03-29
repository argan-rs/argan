use std::{
	convert::Infallible,
	fmt::{Debug, Display},
	future::{ready, Future, Ready},
};

use futures_util::TryFutureExt;
use http::{
	header::{ToStrError, CONTENT_TYPE},
	request::{self, Parts},
	HeaderName, StatusCode,
};
use serde::{
	de::{DeserializeOwned, Error},
	Deserializer,
};

use crate::{
	common::Uncloneable,
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
// PathParams

pub struct PathParams<T>(pub T);

impl<'de, T> PathParams<T>
where
	T: DeserializeOwned,
{
	pub fn deserialize<D: Deserializer<'de>>(&mut self, deserializer: D) -> Result<(), D::Error> {
		self.0 = T::deserialize(deserializer)?;

		Ok(())
	}
}

impl<'n, HE, T> FromRequestHead<Args<'n, HE>> for PathParams<T>
where
	HE: Sync,
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
	HE: Sync,
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
	HE: Sync,
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
	HE: Sync,
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
// Remaining path

pub enum RemainingPath {
	Value(Box<str>),
	None,
}

impl<'n, HE> FromRequestHead<Args<'n, HE>> for RemainingPath
where
	HE: Sync,
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
	HE: Sync,
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
