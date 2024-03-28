use std::{convert::Infallible, future::Future};

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
		args: &mut Args,
	) -> impl Future<Output = Result<Self, Self::Error>> + Send;
}

// --------------------------------------------------
// FromRequest<B> trait

pub trait FromRequest<B, Args>: Sized {
	type Error: Into<BoxedErrorResponse>;

	fn from_request(
		request: Request<B>,
		args: &mut Args,
	) -> impl Future<Output = Result<Self, Self::Error>> + Send;
}

impl<B, Args> FromRequest<B, Args> for Request<B>
where
	B: Send,
	Args: Send,
{
	type Error = Infallible;

	async fn from_request(request: Request<B>, _args: &mut Args) -> Result<Self, Self::Error> {
		Ok(request)
	}
}

// --------------------------------------------------------------------------------
