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
	body::Body,
	common::{BoxedError, IntoArray, Uncloneable},
	data::header::ContentTypeError,
	handler::Args,
	pattern,
	response::{BoxedErrorResponse, IntoResponse, IntoResponseHead, Response},
	routing::RoutingState,
	ImplError, StdError,
};

// ----------

pub use http::{Method, Uri, Version};

// --------------------------------------------------

pub mod websocket;

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

pub type Request<B = Body> = http::Request<B>;
pub type RequestHead = Parts;

// --------------------------------------------------------------------------------
// FromRequestHead trait

pub trait FromRequestHead<Ext>: Sized {
	type Error: Into<BoxedErrorResponse>;

	fn from_request_head(
		head: &mut RequestHead,
		args: &mut Args<'_, Ext>,
	) -> impl Future<Output = Result<Self, Self::Error>> + Send;
}

// --------------------------------------------------------------------------------
// FromRequest<B> trait

pub trait FromRequest<B, Ext>: Sized {
	type Error: Into<BoxedErrorResponse>;

	fn from_request(
		request: Request<B>,
		args: &mut Args<'_, Ext>,
	) -> impl Future<Output = Result<Self, Self::Error>> + Send;
}

impl<B, Ext> FromRequest<B, Ext> for Request<B>
where
	B: Send,
	Ext: Sync,
{
	type Error = Infallible;

	async fn from_request(
		request: Request<B>,
		_args: &mut Args<'_, Ext>,
	) -> Result<Self, Self::Error> {
		Ok(request)
	}
}

// --------------------------------------------------------------------------------

// --------------------------------------------------

impl<Ext, T, E> FromRequestHead<Ext> for Result<T, E>
where
	Ext: Sync,
	T: FromRequestHead<Ext, Error = E>,
{
	type Error = Infallible;

	async fn from_request_head(
		head: &mut RequestHead,
		args: &mut Args<'_, Ext>,
	) -> Result<Self, Self::Error> {
		let result = T::from_request_head(head, args).await;

		Ok(result)
	}
}

impl<B, Ext, T, E> FromRequest<B, Ext> for Result<T, E>
where
	B: Send,
	Ext: Sync,
	T: FromRequest<B, Ext, Error = E>,
{
	type Error = Infallible;

	async fn from_request(
		request: Request<B>,
		args: &mut Args<'_, Ext>,
	) -> Result<Self, Self::Error> {
		let result = T::from_request(request, args).await;

		Ok(result)
	}
}

// --------------------------------------------------
// RequestHead

// impl<B> FromRequest<B> for RequestHead
// where
// 	B: Send,
// {
// 	type Error = Infallible;
//
// 	#[inline]
// 	async fn from_request(request: Request<B>) -> Result<Self, Self::Error> {
// 	  let (head, _) = request.into_parts();
//
// 		Ok(head)
// 	}
// }

// --------------------------------------------------
// Method

impl<Ext: Sync> FromRequestHead<Ext> for Method {
	type Error = Infallible;

	async fn from_request_head(
		head: &mut RequestHead,
		_args: &mut Args<'_, Ext>,
	) -> Result<Self, Self::Error> {
		Ok(head.method.clone())
	}
}

impl<B, Ext> FromRequest<B, Ext> for Method
where
	B: Send,
	Ext: Sync,
{
	type Error = Infallible;

	async fn from_request(
		request: Request<B>,
		_args: &mut Args<'_, Ext>,
	) -> Result<Self, Self::Error> {
		let (head, _) = request.into_parts();

		Ok(head.method)
	}
}

// ----------

impl IntoArray<Method, 1> for Method {
	fn into_array(self) -> [Method; 1] {
		[self]
	}
}

// --------------------------------------------------
// Uri

impl<Ext: Sync> FromRequestHead<Ext> for Uri {
	type Error = Infallible;

	async fn from_request_head(
		head: &mut RequestHead,
		_args: &mut Args<'_, Ext>,
	) -> Result<Self, Self::Error> {
		Ok(head.uri.clone())
	}
}

impl<B, Ext> FromRequest<B, Ext> for Uri
where
	B: Send,
	Ext: Sync,
{
	type Error = Infallible;

	async fn from_request(
		request: Request<B>,
		_args: &mut Args<'_, Ext>,
	) -> Result<Self, Self::Error> {
		let (head, _) = request.into_parts();

		Ok(head.uri)
	}
}

// --------------------------------------------------
// Version

impl<Ext: Sync> FromRequestHead<Ext> for Version {
	type Error = Infallible;

	async fn from_request_head(
		head: &mut RequestHead,
		_args: &mut Args<'_, Ext>,
	) -> Result<Self, Self::Error> {
		Ok(head.version)
	}
}

impl<B, E> FromRequest<B, E> for Version
where
	B: Send,
	E: Sync,
{
	type Error = Infallible;

	async fn from_request(request: Request<B>, _args: &mut Args<'_, E>) -> Result<Self, Self::Error> {
		let (head, _) = request.into_parts();

		Ok(head.version)
	}
}

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

impl<Ext, T> FromRequestHead<Ext> for PathParams<T>
where
	Ext: Sync,
	T: DeserializeOwned,
{
	type Error = PathParamsError;

	async fn from_request_head(
		head: &mut RequestHead,
		args: &mut Args<'_, Ext>,
	) -> Result<Self, Self::Error> {
		let mut from_params_list = args.routing_state.uri_params.deserializer();

		T::deserialize(&mut from_params_list)
			.map(|value| Self(value))
			.map_err(Into::into)
	}
}

impl<B, Ext, T> FromRequest<B, Ext> for PathParams<T>
where
	B: Send,
	Ext: Sync,
	T: DeserializeOwned,
{
	type Error = PathParamsError;

	async fn from_request(
		request: Request<B>,
		_args: &mut Args<'_, Ext>,
	) -> Result<Self, Self::Error> {
		let (mut head, _) = request.into_parts();

		Self::from_request_head(&mut head, _args).await
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

impl<Ext, T> FromRequestHead<Ext> for QueryParams<T>
where
	Ext: Sync,
	T: DeserializeOwned,
{
	type Error = QueryParamsError;

	async fn from_request_head(
		head: &mut RequestHead,
		_args: &mut Args<'_, Ext>,
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

impl<B, Ext, T> FromRequest<B, Ext> for QueryParams<T>
where
	B: Send,
	Ext: Sync,
	T: DeserializeOwned,
{
	type Error = QueryParamsError;

	async fn from_request(
		request: Request<B>,
		_args: &mut Args<'_, Ext>,
	) -> Result<Self, Self::Error> {
		let (mut head, _) = request.into_parts();

		Self::from_request_head(&mut head, _args).await
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

impl<Ext> FromRequestHead<Ext> for RemainingPath
where
	Ext: Sync,
{
	type Error = Infallible;

	async fn from_request_head(
		head: &mut RequestHead,
		args: &mut Args<'_, Ext>,
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

impl<B, Ext> FromRequest<B, Ext> for RemainingPath
where
	B: Send,
	Ext: Sync,
{
	type Error = Infallible;

	async fn from_request(
		request: Request<B>,
		args: &mut Args<'_, Ext>,
	) -> Result<Self, Self::Error> {
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

macro_rules! impl_extractions_for_tuples {
	($t1:ident, $(($($t:ident),*),)? $tl:ident) => {
		#[allow(non_snake_case)]
		impl<$t1, $($($t,)*)? $tl, E> FromRequestHead<E> for ($t1, $($($t,)*)? $tl)
		where
			$t1: FromRequestHead<E> + Send,
			$($($t: FromRequestHead<E> + Send,)*)?
			$tl: FromRequestHead<E> + Send,
			E: Sync,
		{
			type Error = BoxedErrorResponse;

			async fn from_request_head(
				head: &mut RequestHead,
				args: &mut Args<'_, E>,
			) -> Result<Self, Self::Error> {
				let $t1 = $t1::from_request_head(head, args).await.map_err(Into::into)?;

				$(
					$(
						let $t = $t::from_request_head(head, args).await.map_err(Into::into)?;
					)*
				)?

				let $tl = $tl::from_request_head(head, args).await.map_err(Into::into)?;

				Ok(($t1, $($($t,)*)? $tl))
			}
		}

		#[allow(non_snake_case)]
		impl<$t1, $($($t,)*)? $tl, B, E> FromRequest<B, E> for ($t1, $($($t,)*)? $tl)
		where
			$t1: FromRequestHead<E> + Send,
			$($($t: FromRequestHead<E> + Send,)*)?
			$tl: FromRequest<B, E> + Send,
			B: Send,
			E: Sync,
		{
			type Error = BoxedErrorResponse;

			async fn from_request(
				request: Request<B>,
				args: &mut Args<'_, E>,
			) -> Result<Self, Self::Error> {
				let (mut head, body) = request.into_parts();

				let $t1 = $t1::from_request_head(&mut head, args).await.map_err(Into::into)?;

				$($(
					let $t = $t::from_request_head(&mut head, args).await.map_err(Into::into)?;
				)*)?

				let request = Request::from_parts(head, body);

				let $tl = $tl::from_request(request, args).await.map_err(Into::into)?;

				Ok(($t1, $($($t,)*)? $tl))
			}
		}
	};
}

call_for_tuples!(impl_extractions_for_tuples!);

// --------------------------------------------------------------------------------
