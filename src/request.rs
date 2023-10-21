use std::{
	convert::Infallible,
	future::{ready, Future, Ready},
};

use http::{request::Parts, Method, Uri};
use serde::{de::DeserializeOwned, Deserializer};

use crate::{
	body::IncomingBody,
	response::{IntoResponse, Response},
	routing::RoutingState,
};

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

pub type Request<B = IncomingBody> = http::Request<B>;
pub type Head = Parts;

// --------------------------------------------------------------------------------
// FromRequestHead trait

pub trait FromRequestHead: Sized {
	type Error: IntoResponse;
	type Future: Future<Output = Result<Self, Self::Error>>;

	fn from_request_head(head: &mut Head) -> Self::Future;
}

// --------------------------------------------------
// FromRequest<B> trait

pub trait FromRequest<B>: Sized {
	type Error: IntoResponse;
	type Future: Future<Output = Result<Self, Self::Error>>;

	fn from_request(request: Request<B>) -> Self::Future;
}

impl<B> FromRequest<B> for Request<B> {
	type Error = Response;
	type Future = Ready<Result<Self, Self::Error>>;

	fn from_request(request: Request<B>) -> Self::Future {
		ready(Ok(request))
	}
}

impl<T: FromRequestHead, B> FromRequest<B> for T {
	type Error = T::Error;
	type Future = T::Future;

	fn from_request(request: Request<B>) -> Self::Future {
		let (mut head, _) = request.into_parts();

		T::from_request_head(&mut head)
	}
}

// --------------------------------------------------
// Method & Uri

impl FromRequestHead for Method {
	type Error = Infallible;
	type Future = Ready<Result<Self, Self::Error>>;

	fn from_request_head(head: &mut Head) -> Self::Future {
		ready(Ok(head.method.clone()))
	}
}

impl FromRequestHead for Uri {
	type Error = Infallible;
	type Future = Ready<Result<Self, Self::Error>>;

	fn from_request_head(head: &mut Head) -> Self::Future {
		ready(Ok(head.uri.clone()))
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

impl<'de, T> FromRequestHead for PathParam<T>
where
	T: DeserializeOwned,
{
	type Error = Infallible;
	type Future = Ready<Result<Self, Self::Error>>;

	fn from_request_head(head: &mut Head) -> Self::Future {
		let mut routing_state = head.extensions.get_mut::<RoutingState>().unwrap();
		let mut from_params_list = routing_state.path_params.deserializer();

		let value = T::deserialize(&mut from_params_list).unwrap();

		ready(Ok(Self(value)))
	}
}

// --------------------------------------------------
// QueryParams

pub struct QueryParams<T>(pub T);

impl<'de, T> FromRequestHead for QueryParams<T>
where
	T: DeserializeOwned,
{
	type Error = Infallible;
	type Future = Ready<Result<Self, Self::Error>>;

	fn from_request_head(head: &mut Head) -> Self::Future {
		let query_string = head.uri.query().unwrap();

		let value = serde_urlencoded::from_str::<T>(query_string).unwrap();

		ready(Ok(Self(value)))
	}
}

// --------------------------------------------------
