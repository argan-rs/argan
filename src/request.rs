use std::{
	convert::Infallible,
	error::Error,
	ffi::FromBytesUntilNulError,
	fmt::Display,
	future::{ready, Future, Ready},
};

use futures_util::TryFutureExt;
use http::{
	header::{ToStrError, CONTENT_TYPE},
	request::{self, Parts},
	HeaderName, StatusCode,
};
use serde::{de::DeserializeOwned, Deserializer};

use crate::{
	body::IncomingBody,
	common::{IntoArray, Uncloneable},
	header::HeaderError,
	response::{IntoResponse, IntoResponseHead, Response},
	routing::RoutingState,
	ImplError, StdError,
};

// ----------

pub use http::{Method, Uri, Version};

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

pub type Request<B = IncomingBody> = http::Request<B>;
pub type RequestHead = Parts;

// --------------------------------------------------------------------------------
// FromRequestHead trait

pub trait FromRequestHead: Sized {
	type Error: IntoResponse;

	fn from_request_head(
		head: &mut RequestHead,
	) -> impl Future<Output = Result<Self, Self::Error>> + Send;
}

// --------------------------------------------------------------------------------
// FromRequest<B> trait

pub trait FromRequest<B>: Sized {
	type Error: IntoResponse;

	fn from_request(request: Request<B>) -> impl Future<Output = Result<Self, Self::Error>> + Send;
}

impl<B> FromRequest<B> for Request<B>
where
	B: Send,
{
	type Error = Infallible;

	async fn from_request(request: Request<B>) -> Result<Self, Self::Error> {
		Ok(request)
	}
}

// --------------------------------------------------------------------------------

pub(crate) fn content_type<B>(request: &Request<B>) -> Result<&str, HeaderError> {
	let content_type = request
		.headers()
		.get(CONTENT_TYPE)
		.ok_or(HeaderError::MissingHeader(CONTENT_TYPE))?;

	content_type.to_str().map_err(Into::into)
}

// --------------------------------------------------------------------------------

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

impl FromRequestHead for Method {
	type Error = Infallible;

	async fn from_request_head(head: &mut RequestHead) -> Result<Self, Self::Error> {
		Ok(head.method.clone())
	}
}

impl<B> FromRequest<B> for Method
where
	B: Send,
{
	type Error = Infallible;

	async fn from_request(request: Request<B>) -> Result<Self, Self::Error> {
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

impl FromRequestHead for Uri {
	type Error = Infallible;

	async fn from_request_head(head: &mut RequestHead) -> Result<Self, Self::Error> {
		Ok(head.uri.clone())
	}
}

impl<B> FromRequest<B> for Uri
where
	B: Send,
{
	type Error = Infallible;

	async fn from_request(request: Request<B>) -> Result<Self, Self::Error> {
		let (head, _) = request.into_parts();

		Ok(head.uri)
	}
}

// --------------------------------------------------
// Version

impl FromRequestHead for Version {
	type Error = Infallible;

	async fn from_request_head(head: &mut RequestHead) -> Result<Self, Self::Error> {
		Ok(head.version)
	}
}

impl<B> FromRequest<B> for Version
where
	B: Send,
{
	type Error = Infallible;

	async fn from_request(request: Request<B>) -> Result<Self, Self::Error> {
		let (head, _) = request.into_parts();

		Ok(head.version)
	}
}

// --------------------------------------------------
// PathParam

pub struct PathParam<T>(pub T);

impl<'de, T> PathParam<T>
where
	T: DeserializeOwned,
{
	pub fn deserialize<D: Deserializer<'de>>(&mut self, deserializer: D) -> Result<(), D::Error> {
		self.0 = T::deserialize(deserializer)?;

		Ok(())
	}
}

impl<T> FromRequestHead for PathParam<T>
where
	T: DeserializeOwned,
{
	type Error = StatusCode; // TODO.

	async fn from_request_head(head: &mut RequestHead) -> Result<Self, Self::Error> {
		let routing_state = head
			.extensions
			.get_mut::<Uncloneable<RoutingState>>()
			.expect("Uncloneable<RoutingState> should be inserted before request_handler is called")
			.as_mut()
			.expect("RoutingState should always exist in Uncloneable");

		let mut from_params_list = routing_state.path_params.deserializer();

		T::deserialize(&mut from_params_list)
			.map(|value| Self(value))
			.map_err(|_| StatusCode::NOT_FOUND)
	}
}

impl<B, T> FromRequest<B> for PathParam<T>
where
	B: Send,
	T: DeserializeOwned,
{
	type Error = StatusCode; // TODO.

	async fn from_request(request: Request<B>) -> Result<Self, Self::Error> {
		let (mut head, _) = request.into_parts();

		Self::from_request_head(&mut head).await
	}
}

// --------------------------------------------------
// QueryParams

pub struct QueryParams<T>(pub T);

impl<T> FromRequestHead for QueryParams<T>
where
	T: DeserializeOwned,
{
	type Error = StatusCode; // TODO.

	async fn from_request_head(head: &mut RequestHead) -> Result<Self, Self::Error> {
		let query_string = head.uri.query().ok_or(StatusCode::BAD_REQUEST)?;

		serde_urlencoded::from_str::<T>(query_string)
			.map(|value| Self(value))
			.map_err(|_| StatusCode::BAD_REQUEST)
	}
}

impl<B, T> FromRequest<B> for QueryParams<T>
where
	B: Send,
	T: DeserializeOwned,
{
	type Error = StatusCode; // TODO.

	async fn from_request(request: Request<B>) -> Result<Self, Self::Error> {
		let (mut head, _) = request.into_parts();

		Self::from_request_head(&mut head).await
	}
}

// --------------------------------------------------------------------------------

macro_rules! impl_extractions_for_tuples {
	($t1:ident, $(($($t:ident),*),)? $tl:ident) => {
		#[allow(non_snake_case)]
		impl<$t1, $($($t,)*)? $tl> FromRequestHead for ($t1, $($($t,)*)? $tl)
		where
			$t1: FromRequestHead + Send,
			$($($t: FromRequestHead + Send,)*)?
			$tl: FromRequestHead + Send,
		{
			type Error = Response;

			async fn from_request_head(head: &mut RequestHead) -> Result<Self, Self::Error> {
				let $t1 = $t1::from_request_head(head).await.map_err(|error| error.into_response())?;

				$($(let $t = $t::from_request_head(head).await.map_err(|error| error.into_response())?;)*)?

				let $tl = $tl::from_request_head(head).await.map_err(|error| error.into_response())?;

				Ok(($t1, $($($t,)*)? $tl))
			}
		}

		#[allow(non_snake_case)]
		impl<$t1, $($($t,)*)? $tl, B> FromRequest<B> for ($t1, $($($t,)*)? $tl)
		where
			$t1: FromRequestHead + Send,
			$($($t: FromRequestHead + Send,)*)?
			$tl: FromRequest<B> + Send,
			B: Send,
		{
			type Error = Response;

			async fn from_request(request: Request<B>) -> Result<Self, Self::Error> {
				let (mut head, body) = request.into_parts();

				let $t1 = $t1::from_request_head(&mut head).await.map_err(|error| error.into_response())?;

				$($(
					let $t = $t::from_request_head(&mut head).await.map_err(|error| error.into_response())?;
				)*)?

				let request = Request::from_parts(head, body);

				let $tl = $tl::from_request(request).await.map_err(|error| error.into_response())?;

				Ok(($t1, $($($t,)*)? $tl))
			}
		}
	};
}

call_for_tuples!(impl_extractions_for_tuples!);

// --------------------------------------------------------------------------------
