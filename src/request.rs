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
	body::Body,
	common::{BoxedError, IntoArray, Uncloneable},
	handler::Args,
	header::ContentTypeError,
	response::{BoxedErrorResponse, IntoResponse, IntoResponseHead, Response},
	routing::RoutingState,
	ImplError, StdError,
};

// ----------

pub use http::{Method, Uri, Version};

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

pub type Request<B = Body> = http::Request<B>;
pub type RequestHead = Parts;

// --------------------------------------------------------------------------------
// FromRequestHead trait

pub trait FromRequestHead<E>: Sized {
	type Error: Into<BoxedErrorResponse>;

	fn from_request_head(
		head: &mut RequestHead,
		args: &mut Args<'_, E>,
	) -> impl Future<Output = Result<Self, Self::Error>> + Send;
}

// --------------------------------------------------------------------------------
// FromRequest<B> trait

pub trait FromRequest<B, E>: Sized {
	type Error: Into<BoxedErrorResponse>;

	fn from_request(
		request: Request<B>,
		args: &mut Args<'_, E>,
	) -> impl Future<Output = Result<Self, Self::Error>> + Send;
}

impl<B, E> FromRequest<B, E> for Request<B>
where
	B: Send,
	E: Sync,
{
	type Error = Infallible;

	async fn from_request(request: Request<B>, _args: &mut Args<'_, E>) -> Result<Self, Self::Error> {
		Ok(request)
	}
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

impl<E: Sync> FromRequestHead<E> for Method {
	type Error = Infallible;

	async fn from_request_head(
		head: &mut RequestHead,
		_args: &mut Args<'_, E>,
	) -> Result<Self, Self::Error> {
		Ok(head.method.clone())
	}
}

impl<B, E> FromRequest<B, E> for Method
where
	B: Send,
	E: Sync,
{
	type Error = Infallible;

	async fn from_request(request: Request<B>, _args: &mut Args<'_, E>) -> Result<Self, Self::Error> {
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

impl<E: Sync> FromRequestHead<E> for Uri {
	type Error = Infallible;

	async fn from_request_head(
		head: &mut RequestHead,
		_args: &mut Args<'_, E>,
	) -> Result<Self, Self::Error> {
		Ok(head.uri.clone())
	}
}

impl<B, E> FromRequest<B, E> for Uri
where
	B: Send,
	E: Sync,
{
	type Error = Infallible;

	async fn from_request(request: Request<B>, _args: &mut Args<'_, E>) -> Result<Self, Self::Error> {
		let (head, _) = request.into_parts();

		Ok(head.uri)
	}
}

// --------------------------------------------------
// Version

impl<E: Sync> FromRequestHead<E> for Version {
	type Error = Infallible;

	async fn from_request_head(
		head: &mut RequestHead,
		_args: &mut Args<'_, E>,
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

// impl<E, T> FromRequestHead<E> for PathParam<T>
// where
// 	E: Sync,
// 	T: DeserializeOwned,
// {
// 	type Error = StatusCode; // TODO.
//
// 	async fn from_request_head(
// 		head: &mut RequestHead,
// 		_args: &mut Args<'_, E>,
// 	) -> Result<Self, Self::Error> {
// 		let routing_state = head
// 			.extensions
// 			.get_mut::<Uncloneable<RoutingState>>()
// 			.expect("Uncloneable<RoutingState> should be inserted before request_handler is called")
// 			.as_mut()
// 			.expect("RoutingState should always exist in Uncloneable");
//
// 		let mut from_params_list = routing_state.uri_params.deserializer();
//
// 		T::deserialize(&mut from_params_list)
// 			.map(|value| Self(value))
// 			.map_err(|_| StatusCode::NOT_FOUND)
// 	}
// }
//
// impl<B, E, T> FromRequest<B, E> for PathParam<T>
// where
// 	B: Send,
// 	E: Sync,
// 	T: DeserializeOwned,
// {
// 	type Error = StatusCode; // TODO.
//
// 	async fn from_request(request: Request<B>, _args: &mut Args<'_, E>) -> Result<Self, Self::Error> {
// 		let (mut head, _) = request.into_parts();
//
// 		Self::from_request_head(&mut head, _args).await
// 	}
// }

// --------------------------------------------------
// QueryParams

// pub struct QueryParams<T>(pub T);
//
// impl<E, T> FromRequestHead<E> for QueryParams<T>
// where
// 	E: Sync,
// 	T: DeserializeOwned,
// {
// 	type Error = StatusCode; // TODO.
//
// 	async fn from_request_head(
// 		head: &mut RequestHead,
// 		_args: &mut Args<'_, E>,
// 	) -> Result<Self, Self::Error> {
// 		let query_string = head.uri.query().ok_or(StatusCode::BAD_REQUEST)?;
//
// 		serde_urlencoded::from_str::<T>(query_string)
// 			.map(|value| Self(value))
// 			.map_err(|_| StatusCode::BAD_REQUEST)
// 	}
// }
//
// impl<B, E, T> FromRequest<B, E> for QueryParams<T>
// where
// 	B: Send,
// 	E: Sync,
// 	T: DeserializeOwned,
// {
// 	type Error = StatusCode; // TODO.
//
// 	async fn from_request(request: Request<B>, _args: &mut Args<'_, E>) -> Result<Self, Self::Error> {
// 		let (mut head, _) = request.into_parts();
//
// 		Self::from_request_head(&mut head, _args).await
// 	}
// }

// --------------------------------------------------
// Remaining path

pub enum RemainingPath {
	Value(Box<str>),
	None,
}

impl<E> FromRequestHead<E> for RemainingPath
where
	E: Sync,
{
	type Error = Infallible;

	async fn from_request_head(
		head: &mut RequestHead,
		args: &mut Args<'_, E>,
	) -> Result<Self, Self::Error> {
		args
			.routing_state
			.path_traversal
			.remaining_segments(head.uri.path())
			.map_or(Ok(RemainingPath::None), |remaining_path| {
				Ok(RemainingPath::Value(remaining_path.into()))
			})
	}
}

impl<B, E> FromRequest<B, E> for RemainingPath
where
	B: Send,
	E: Sync,
{
	type Error = Infallible;

	async fn from_request(request: Request<B>, args: &mut Args<'_, E>) -> Result<Self, Self::Error> {
		args
			.routing_state
			.path_traversal
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
