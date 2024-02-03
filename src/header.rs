use std::convert::Infallible;

use crate::{
	handler::Args,
	request::{FromRequest, FromRequestHead, Request, RequestHead},
	response::{IntoResponse, IntoResponseHead, Response, ResponseHead},
	ImplError,
};

// ----------

pub use http::header::*;

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

impl<E: Sync> FromRequestHead<E> for HeaderMap {
	type Error = Infallible;

	async fn from_request_head(
		head: &mut RequestHead,
		_args: &mut Args<'_, E>,
	) -> Result<Self, Self::Error> {
		Ok(head.headers.clone())
	}
}

impl<B, E> FromRequest<B, E> for HeaderMap
where
	B: Send,
	E: Sync,
{
	type Error = Infallible;

	async fn from_request(request: Request<B>, _args: &mut Args<'_, E>) -> Result<Self, Self::Error> {
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
