use std::{
	convert::Infallible,
	future::{ready, Future},
};

use http::request::Parts;

use crate::{body::Body, response::BoxedErrorResponse};

// ----------

pub use http::{Method, Uri, Version};

// --------------------------------------------------

mod impls;

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

pub type Request<B = Body> = http::Request<B>;
pub type RequestHead = Parts;

// --------------------------------------------------------------------------------

// --------------------------------------------------
// FromRequestHead trait

pub trait FromRequestHead<Args>: Sized {
	type Error: Into<BoxedErrorResponse>;

	fn from_request_head(
		head: &mut RequestHead,
		args: &Args,
	) -> impl Future<Output = Result<Self, Self::Error>> + Send;
}

// --------------------------------------------------
// FromRequest<B> trait

pub trait FromRequest<B, Args>: Sized {
	type Error: Into<BoxedErrorResponse>;

	fn from_request(
		request: Request<B>,
		args: Args,
	) -> impl Future<Output = Result<Self, Self::Error>> + Send;
}

impl<B, Args> FromRequest<B, Args> for Request<B>
where
	B: Send,
{
	type Error = Infallible;

	fn from_request(
		request: Request<B>,
		_args: Args,
	) -> impl Future<Output = Result<Self, Self::Error>> {
		ready(Ok(request))
	}
}

// --------------------------------------------------------------------------------
