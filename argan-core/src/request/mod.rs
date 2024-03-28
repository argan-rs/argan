use std::{convert::Infallible, future::Future};

use http::request::Parts;

use crate::{body::Body, response::BoxedErrorResponse, Args};

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

pub trait FromRequestHead<PE, HE>: Sized {
	type Error: Into<BoxedErrorResponse>;

	fn from_request_head(
		head: &mut RequestHead,
		args: &mut Args<'_, PE, HE>,
	) -> impl Future<Output = Result<Self, Self::Error>> + Send;
}

// --------------------------------------------------
// FromRequest<B> trait

pub trait FromRequest<B, PE, HE>: Sized {
	type Error: Into<BoxedErrorResponse>;

	fn from_request(
		request: Request<B>,
		args: &mut Args<'_, PE, HE>,
	) -> impl Future<Output = Result<Self, Self::Error>> + Send;
}

impl<B, PE, HE> FromRequest<B, PE, HE> for Request<B>
where
	B: Send,
	PE: Send,
	HE: Sync,
{
	type Error = Infallible;

	async fn from_request(
		request: Request<B>,
		_args: &mut Args<'_, PE, HE>,
	) -> Result<Self, Self::Error> {
		Ok(request)
	}
}

// --------------------------------------------------------------------------------
