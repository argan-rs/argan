use std::convert::Infallible;

use crate::{
	request::{FromRequest, FromRequestHead, Request, RequestHead},
	response::{IntoResponse, IntoResponseHead, Response, ResponseHead},
	ImplError,
};

// ----------

pub use http::header::*;

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

impl FromRequestHead for HeaderMap {
	type Error = Infallible;

	async fn from_request_head(head: &mut RequestHead) -> Result<Self, Self::Error> {
		Ok(head.headers.clone())
	}
}

impl<B> FromRequest<B> for HeaderMap
where
	B: Send,
{
	type Error = Infallible;

	async fn from_request(request: Request<B>) -> Result<Self, Self::Error> {
		let (RequestHead { headers, .. }, _) = request.into_parts();

		Ok(headers)
	}
}

// -------------------------

impl IntoResponseHead for HeaderMap {
	type Error = Infallible;

	#[inline]
	fn into_response_head(self, mut head: ResponseHead) -> Result<ResponseHead, Self::Error> {
		head.headers.extend(self);

		Ok(head)
	}
}

impl IntoResponse for HeaderMap {
	fn into_response(self) -> Response {
		let mut response = ().into_response();
		*response.headers_mut() = self;

		response
	}
}

// --------------------------------------------------

#[derive(Debug, ImplError)]
pub(crate) enum HeaderError {
	#[error("missing {0} header")]
	MissingHeader(HeaderName),
	#[error(transparent)]
	InvalidValue(#[from] ToStrError),
}

// --------------------------------------------------------------------------------
