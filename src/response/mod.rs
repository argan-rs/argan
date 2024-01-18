use std::{any::Any, convert::Infallible};

use http::response::Parts;

use crate::{
	body::{Body, BodyExt, BoxedBody, Bytes},
	request::FromRequestHead,
	utils::{BoxedError, SCOPE_VALIDITY},
};

// ----------

pub use http::StatusCode;

// --------------------------------------------------

pub mod stream;

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

pub type Response<B = BoxedBody> = http::response::Response<B>;
pub type ResponseHead = Parts;

// --------------------------------------------------
// IntoResponseHead trait

pub trait IntoResponseHead {
	type Error: IntoResponse;

	fn into_response_head(self, head: ResponseHead) -> Result<ResponseHead, Self::Error>;
}

// --------------------------------------------------
// IntoResponse trait

pub trait IntoResponse {
	fn into_response(self) -> Response;
}

impl<B> IntoResponse for Response<B>
where
	B: Body<Data = Bytes> + Send + Sync + 'static,
	B::Error: Into<BoxedError>,
{
	fn into_response(self) -> Response {
		let (head, body) = self.into_parts();
		let mut some_body = Some(body);

		if let Some(some_boxed_body) = <dyn Any>::downcast_mut::<Option<BoxedBody>>(&mut some_body) {
			let boxed_body = some_boxed_body.take().expect(SCOPE_VALIDITY);

			return Response::from_parts(head, boxed_body);
		}

		let body = some_body.expect(SCOPE_VALIDITY);
		let boxed_body = BoxedBody::new(body.map_err(Into::into));

		Response::from_parts(head, boxed_body)
	}
}

// --------------------------------------------------
// StatusCode

impl IntoResponse for StatusCode {
	#[inline]
	fn into_response(self) -> Response {
		let mut response = Response::default();
		*response.status_mut() = self;

		response
	}
}

// --------------------------------------------------
// Infallible Error

impl IntoResponse for Infallible {
	#[inline]
	fn into_response(self) -> Response {
		Response::default()
	}
}

// --------------------------------------------------
// Unit ()

impl IntoResponse for () {
	#[inline]
	fn into_response(self) -> Response {
		Response::default()
	}
}

// --------------------------------------------------
// Option<T>

impl<T: IntoResponse> IntoResponse for Option<T> {
	#[inline]
	fn into_response(self) -> Response {
		match self {
			Some(value) => value.into_response(),
			None => {
				let mut response = Response::default();
				*response.status_mut() = StatusCode::NO_CONTENT;

				response
			}
		}
	}
}

// --------------------------------------------------
// Result<T, E>

impl<T, E> IntoResponse for Result<T, E>
where
	T: IntoResponse,
	E: IntoResponse,
{
	#[inline]
	fn into_response(self) -> Response {
		match self {
			Ok(value) => value.into_response(),
			Err(error) => error.into_response(),
		}
	}
}

// --------------------------------------------------------------------------------

macro_rules! impl_into_response_for_tuples {
	($t1:ident, $(($($t:ident),*),)? $tl:ident) => {
		#[allow(non_snake_case)]
		impl<$t1, $($($t,)*)? $tl> IntoResponseHead for ($t1, $($($t,)*)? $tl)
		where
			$t1: IntoResponseHead,
			$($($t: IntoResponseHead,)*)?
			$tl: IntoResponseHead,
		{
			type Error = Response;

			fn into_response_head(self, mut head: ResponseHead) -> Result<ResponseHead, Self::Error> {
				let ($t1, $($($t,)*)? $tl) = self;

				head = $t1.into_response_head(head).map_err(|error| error.into_response())?;

				$($(head = $t.into_response_head(head).map_err(|error| error.into_response())?;)*)?

				head = $tl.into_response_head(head).map_err(|error| error.into_response())?;

				Ok(head)
			}
		}

		#[allow(non_snake_case)]
		impl<$($($t,)*)? $tl> IntoResponse for (StatusCode, $($($t,)*)? $tl)
		where
			$($($t: IntoResponseHead,)*)?
			$tl: IntoResponse,
		{
			fn into_response(self) -> Response {
				let (status_code, $($($t,)*)? $tl) = self;

				let (head, body) = $tl.into_response().into_parts();

				$($(
					let head = match $t.into_response_head(head) {
						Ok(head) => head,
						Err(error) => return error.into_response(),
					};
				)*)?

				let mut response = Response::from_parts(head, body);
				*response.status_mut() = status_code;

				response
			}
		}

		#[allow(non_snake_case)]
		impl<$t1, $($($t,)*)? $tl> IntoResponse for ($t1, $($($t,)*)? $tl)
		where
			$t1: IntoResponseHead,
			$($($t: IntoResponseHead,)*)?
			$tl: IntoResponse,
		{
			fn into_response(self) -> Response {
				let ($t1, $($($t,)*)? $tl) = self;

				let (head, body) = $tl.into_response().into_parts();

				let head = match $t1.into_response_head(head) {
					Ok(head) => head,
					Err(error) => return error.into_response(),
				};

				$($(
					let head = match $t.into_response_head(head) {
						Ok(head) => head,
						Err(error) => return error.into_response(),
					};
				)*)?

				Response::from_parts(head, body)
			}
		}
	};
}

call_for_tuples!(impl_into_response_for_tuples!);

// --------------------------------------------------------------------------------
