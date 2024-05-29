use std::convert::Infallible;

use futures_util::FutureExt;

use super::*;

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

// --------------------------------------------------
// Result<T, E>

impl<B, T, E> FromRequest<B> for Result<T, E>
where
	T: FromRequest<B, Error = E>,
{
	type Error = Infallible;

	fn from_request(
		head_parts: &mut RequestHeadParts,
		body: B,
	) -> impl Future<Output = Result<Self, Self::Error>> {
		T::from_request(head_parts, body).map(Ok)
	}
}

// --------------------------------------------------
// Option<T>

impl<B, T> FromRequest<B> for Option<T>
where
	T: FromRequest<B>,
{
	type Error = Infallible;

	fn from_request(
		head_parts: &mut RequestHeadParts,
		body: B,
	) -> impl Future<Output = Result<Self, Self::Error>> {
		T::from_request(head_parts, body).map(|result| Ok(result.ok()))
	}
}

// --------------------------------------------------
// ()

impl<B> FromRequest<B> for () {
	type Error = Infallible;

	fn from_request(
		_head_parts: &mut RequestHeadParts,
		_: B,
	) -> impl Future<Output = Result<Self, Self::Error>> + Send {
		ready(Ok(()))
	}
}

// --------------------------------------------------------------------------------
