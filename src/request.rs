use std::{
	convert::Infallible,
	future::{ready, Future, Ready},
};

use hyper::{http::request::Parts, HeaderMap, Method, Uri, Version};

use crate::{
	body::IncomingBody,
	response::{IntoResponse, Response},
};

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

pub type Request<B = IncomingBody> = hyper::Request<B>;
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

pub trait FromRequest<B>: Sized {
	type Error: IntoResponse;
	type Future: Future<Output = Result<Self, Self::Error>>;

	fn from_request(req: Request<B>) -> Self::Future;
}

impl<B> FromRequest<B> for Request<B> {
	type Error = Response;
	type Future = Ready<Result<Self, Self::Error>>;

	fn from_request(req: Request<B>) -> Self::Future {
		ready(Ok(req))
	}
}

// -------------------------
