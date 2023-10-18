use std::{
	convert::Infallible,
	fmt::Debug,
	future::{ready, Future, Ready},
	marker::PhantomData,
	pin::pin,
	task::Poll,
};

use http::{request::Parts, HeaderMap, Method, Uri, Version};
use pin_project::pin_project;
use serde::{de::DeserializeOwned, Deserializer};

use crate::{
	body::{Body, BodyExt, IncomingBody},
	response::{IntoResponse, Response},
	routing::RoutingState,
};

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

pub type Request<B = IncomingBody> = http::Request<B>;
pub type Head = Parts;

// --------------------------------------------------------------------------------

pub trait FromRequestHead: Sized {
	type Error: IntoResponse;
	type Future: Future<Output = Result<Self, Self::Error>>;

	fn from_request_head(head: &mut Head) -> Self::Future;
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

impl FromRequestHead for Version {
	type Error = Infallible;
	type Future = Ready<Result<Self, Self::Error>>;

	fn from_request_head(head: &mut Head) -> Self::Future {
		ready(Ok(head.version))
	}
}

impl FromRequestHead for HeaderMap {
	type Error = Infallible;
	type Future = Ready<Result<Self, Self::Error>>;

	fn from_request_head(head: &mut Head) -> Self::Future {
		ready(Ok(head.headers.clone()))
	}
}

// --------------------------------------------------
// Request<B>

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
// Form

pub struct Form<T>(pub T);

impl<'de, B, T> FromRequest<B> for Form<T>
where
	B: Body + 'static,
	B::Error: Debug,
	T: DeserializeOwned,
{
	type Error = Infallible;
	type Future = FormFuture<B, T>;

	fn from_request(request: Request<B>) -> Self::Future {
		FormFuture(request, PhantomData)
	}
}

#[pin_project]
pub struct FormFuture<B, T>(#[pin] Request<B>, PhantomData<T>);

impl<B, T> Future for FormFuture<B, T>
where
	B: Body + 'static,
	B::Error: Debug,
	T: DeserializeOwned,
{
	type Output = Result<Form<T>, Infallible>;

	fn poll(
		self: std::pin::Pin<&mut Self>,
		cx: &mut std::task::Context<'_>,
	) -> std::task::Poll<Self::Output> {
		let self_projection = self.project();
		if let Poll::Ready(result) = pin!(self_projection.0.collect()).poll(cx) {
			let body = result.unwrap().to_bytes();
			let value = serde_urlencoded::from_bytes::<T>(&body).unwrap();

			return Poll::Ready(Ok(Form(value)));
		}

		Poll::Pending
	}
}

// --------------------------------------------------
// Json

pub struct Json<T>(pub T);

impl<'de, B, T> FromRequest<B> for Json<T>
where
	B: Body + 'static,
	B::Error: Debug,
	T: DeserializeOwned,
{
	type Error = Infallible;
	type Future = JsonFuture<B, T>;

	fn from_request(request: Request<B>) -> Self::Future {
		JsonFuture(request, PhantomData)
	}
}

#[pin_project]
pub struct JsonFuture<B, T>(#[pin] Request<B>, PhantomData<T>);

impl<B, T> Future for JsonFuture<B, T>
where
	B: Body + 'static,
	B::Error: Debug,
	T: DeserializeOwned,
{
	type Output = Result<Json<T>, Infallible>;

	fn poll(
		self: std::pin::Pin<&mut Self>,
		cx: &mut std::task::Context<'_>,
	) -> std::task::Poll<Self::Output> {
		let self_projection = self.project();
		if let Poll::Ready(result) = pin!(self_projection.0.collect()).poll(cx) {
			let body = result.unwrap().to_bytes();
			let value = serde_json::from_slice::<T>(&body).unwrap();

			return Poll::Ready(Ok(Json(value)));
		}

		Poll::Pending
	}
}

// --------------------------------------------------
