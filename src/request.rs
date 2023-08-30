use std::future::{ready, Future, Ready};

use hyper::http::request::Parts;

use crate::{
	body::IncomingBody,
	response::{IntoResponse, Response},
};

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

pub type Request<B = IncomingBody> = hyper::Request<B>;

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

pub trait FromRequestParts: Sized {
	type Error: IntoResponse;
	type Future: Future<Output = Result<Self, Self::Error>>;

	fn from_request_parts(parts: &Parts) -> Self::Future;
}

impl<T: FromRequestParts, B> FromRequest<B> for T {
	type Error = T::Error;
	type Future = T::Future;

	fn from_request(req: Request<B>) -> Self::Future {
		let (parts, _) = req.into_parts();

		T::from_request_parts(&parts)
	}
}
