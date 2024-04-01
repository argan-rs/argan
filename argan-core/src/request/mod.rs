use std::{
	convert::Infallible,
	future::{ready, Future},
};

use crate::{body::Body, response::BoxedErrorResponse};

// ----------

pub use http::request::Builder;

// --------------------------------------------------

mod impls;

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

pub type Request<B = Body> = http::request::Request<B>;
pub type RequestHead = http::request::Parts;

// --------------------------------------------------------------------------------

// TODO: Create FromRequest traits with and without args. Create a Request type with routing state.

// --------------------------------------------------
// FromRequestHead trait

// pub trait FromMutRequestHead: Sized {
// 	type Error: Into<BoxedErrorResponse>;
//
// 	fn from_request_head(
// 		head: &mut RequestHead,
// 	) -> impl Future<Output = Result<Self, Self::Error>> + Send;
// }

// --------------------------------------------------
// FromRequestRef trait

pub trait FromRequestRef<'r, B>: Sized {
	type Error: Into<BoxedErrorResponse>;

	fn from_request_ref(
		request: &'r Request<B>,
	) -> impl Future<Output = Result<Self, Self::Error>> + Send;
}

// --------------------------------------------------
// FromRequest<B> trait

pub trait FromRequest<B>: Sized {
	type Error: Into<BoxedErrorResponse>;

	fn from_request(request: Request<B>) -> impl Future<Output = Result<Self, Self::Error>> + Send;
}

impl<B> FromRequest<B> for Request<B>
where
	B: Send,
{
	type Error = Infallible;

	fn from_request(request: Request<B>) -> impl Future<Output = Result<Self, Self::Error>> {
		ready(Ok(request))
	}
}

// --------------------------------------------------------------------------------
