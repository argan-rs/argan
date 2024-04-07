use std::convert::Infallible;

use futures_util::FutureExt;

use super::*;

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

// --------------------------------------------------
// Result<T, E>

impl<'r, B, T, E: 'r> FromRequestRef<'r, B> for Result<T, E>
where
	T: FromRequestRef<'r, B, Error = E>,
{
	type Error = Infallible;

	#[inline(always)]
	fn from_request_ref(request: &'r Request<B>) -> impl Future<Output = Result<Self, Self::Error>> {
		T::from_request_ref(request).map(|result| Ok(result))
	}
}

impl<B, T, E> FromRequest<B> for Result<T, E>
where
	T: FromRequest<B, Error = E>,
{
	type Error = Infallible;

	fn from_request(
		head_parts: &mut RequestHeadParts,
		body: B,
	) -> impl Future<Output = Result<Self, Self::Error>> {
		T::from_request(head_parts, body).map(|result| Ok(result))
	}
}

// --------------------------------------------------
// Option<T>

impl<B, T, E> FromRequest<B> for Option<T>
where
	T: FromRequest<B, Error = E>,
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
