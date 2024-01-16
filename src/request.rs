use std::{
	convert::Infallible,
	future::{ready, Future, Ready},
};

use http::{header::CONTENT_TYPE, request::Parts, StatusCode};
use serde::{de::DeserializeOwned, Deserializer};

use crate::{
	body::IncomingBody,
	response::{IntoResponse, Response},
	routing::RoutingState,
	utils::{IntoArray, Uncloneable},
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

impl<T: FromRequestHead, B> FromRequest<B> for T
where
	B: Send,
{
	type Error = T::Error;

	async fn from_request(request: Request<B>) -> Result<Self, Self::Error> {
		let (mut head, _) = request.into_parts();

		T::from_request_head(&mut head).await
	}
}

// --------------------------------------------------------------------------------

pub(crate) fn content_type<B>(request: &Request<B>) -> Result<&str, StatusCode> {
	let content_type = request
		.headers()
		.get(CONTENT_TYPE)
		.ok_or(StatusCode::BAD_REQUEST)?;

	content_type.to_str().map_err(|e| StatusCode::BAD_REQUEST)
}

// --------------------------------------------------------------------------------
// --------------------------------------------------
// Method

impl FromRequestHead for Method {
	type Error = Infallible;

	async fn from_request_head(head: &mut RequestHead) -> Result<Self, Self::Error> {
		Ok(head.method.clone())
	}
}

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

// --------------------------------------------------
