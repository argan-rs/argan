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

impl<T: IntoResponseHead> IntoResponse for T {
	#[inline]
	fn into_response(self) -> Response {
		let (head, body) = Response::default().into_parts();

		match self.into_response_head(head) {
			Ok(head) => Response::from_parts(head, body),
			Err(error) => error.into_response(),
		}
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

// impl<T1, T2> FromRequestHead for (T1, T2) {
// 	type Error = ;
// }
