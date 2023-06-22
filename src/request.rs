use hyper::http::request::Parts;

use crate::{
	response::{IntoResponse, Response},
	utils::*,
};

use super::body::*;

// --------------------------------------------------

pub type Request<B = Incoming> = hyper::Request<B>;

// --------------------------------------------------

pub trait FromRequest<B>: Sized {
	type Rejection: IntoResponse;
	type Error: Into<BoxedError>;

	fn from_request(req: Request<B>) -> Result<Self, Either<Self::Rejection, Self::Error>>;
}

impl<B> FromRequest<B> for Request<B> {
	type Rejection = Response;
	type Error = BoxedError;

	fn from_request(req: Request<B>) -> Result<Self, Either<Self::Rejection, Self::Error>> {
		Ok(req)
	}
}

// -------------------------

pub trait FromRequestParts: Sized {
	type Rejection: IntoResponse;
	type Error: Into<BoxedError>;

	fn from_request_parts(parts: &Parts) -> Result<Self, Either<Self::Rejection, Self::Error>>;
}

impl<T: FromRequestParts, B> FromRequest<B> for T {
	type Rejection = T::Rejection;
	type Error = T::Error;

	fn from_request(req: Request<B>) -> Result<Self, Either<Self::Rejection, Self::Error>> {
		let (parts, _) = req.into_parts();

		T::from_request_parts(&parts)
	}
}
