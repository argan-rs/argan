use std::convert::Infallible;

use http::StatusCode;

use crate::{
	request::{FromRequest, FromRequestHead, Request, RequestHead},
	response::{IntoResponse, IntoResponseHead, Response, ResponseHead},
};

// ----------

pub use http::Extensions;
// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

// --------------------------------------------------
// Extensions

pub struct Extention<T>(pub T);

impl<T> FromRequestHead for Extention<T>
where
	T: Clone + Send + Sync + 'static,
{
	type Error = StatusCode; // TODO.

	async fn from_request_head(head: &mut RequestHead) -> Result<Self, Self::Error> {
		match head.extensions.get::<T>() {
			Some(value) => Ok(Extention(value.clone())),
			None => Err(StatusCode::INTERNAL_SERVER_ERROR),
		}
	}
}

impl<B, T> FromRequest<B> for Extention<T>
where
	B: Send,
	T: Clone + Send + Sync + 'static,
{
	type Error = StatusCode;

	async fn from_request(request: Request<B>) -> Result<Self, Self::Error> {
		let (mut head, _) = request.into_parts();

		Self::from_request_head(&mut head).await
	}
}

// -------------------------

impl IntoResponseHead for Extensions {
	type Error = Infallible;

	#[inline(always)]
	fn into_response_head(self, mut head: ResponseHead) -> Result<ResponseHead, Self::Error> {
		head.extensions.extend(self);

		Ok(head)
	}
}

impl IntoResponse for Extensions {
	#[inline(always)]
	fn into_response(self) -> Response {
		let mut response = ().into_response();
		*response.extensions_mut() = self;

		response
	}
}

// --------------------------------------------------------------------------------
