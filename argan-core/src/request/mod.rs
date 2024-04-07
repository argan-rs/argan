use std::future::{ready, Future};

use crate::{body::Body, response::BoxedErrorResponse};

// ----------

pub use http::request::Builder;

// --------------------------------------------------

mod impls;

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

pub type Request<B = Body> = http::request::Request<B>;
pub type RequestHeadParts = http::request::Parts;

// --------------------------------------------------
// FromRequestRef

pub trait FromRequestRef<'r, B>: Sized {
	type Error: Into<BoxedErrorResponse>;

	fn from_request_ref(
		request: &'r Request<B>,
	) -> impl Future<Output = Result<Self, Self::Error>> + Send;
}

// --------------------------------------------------
// FromRequest<B>

pub trait FromRequest<B = Body>: Sized {
	type Error: Into<BoxedErrorResponse>;

	fn from_request(
		head_parts: &mut RequestHeadParts,
		body: B,
	) -> impl Future<Output = Result<Self, Self::Error>> + Send;
}

// --------------------------------------------------------------------------------
