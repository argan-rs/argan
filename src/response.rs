use std::{any::Any, borrow::Cow, convert::Infallible};

use http::{request::Parts, Extensions};
use hyper::{
	header::{self, HeaderValue},
	HeaderMap,
};

use crate::{
	body::{Body, BoxedBody, Bytes, Empty, Full},
	routing::StatusCode,
	utils::BoxedError,
};

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

pub type Response<B = BoxedBody> = hyper::http::response::Response<B>;
pub type Head = Parts;

// --------------------------------------------------------------------------------

pub trait IntoResponseHead {
	type Error: IntoResponse;

	fn into_response_head(self, head: Head) -> Result<Head, Self::Error>;
}

impl IntoResponseHead for HeaderMap<HeaderValue> {
	type Error = Infallible;

	#[inline]
	fn into_response_head(self, mut head: Head) -> Result<Head, Self::Error> {
		head.headers.extend(self);

		Ok(head)
	}
}

impl IntoResponseHead for Extensions {
	type Error = Infallible;

	#[inline]
	fn into_response_head(self, mut head: Head) -> Result<Head, Self::Error> {
		head.extensions.extend(self);

		Ok(head)
	}
}

// --------------------------------------------------

pub trait IntoResponse {
	fn into_response(self) -> Response;
}

impl<B> IntoResponse for Response<B>
where
	B: Body<Data = Bytes> + Send + Sync + 'static,
	B::Error: Into<BoxedError>,
{
	fn into_response(self) -> Response {
		let (head, mut body) = self.into_parts();
		let mut some_body = Some(body);

		if let Some(some_boxed_body) = <dyn Any>::downcast_mut::<Option<BoxedBody>>(&mut some_body) {
			let boxed_body = some_boxed_body
				.take()
				.expect("Option should have been created from a valid value in a local scope");

			return Response::from_parts(head, boxed_body);
		}

		let body =
			some_body.expect("Option should have been created from a valid value in a local scope");
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

// -------------------------

impl IntoResponse for Infallible {
	#[inline]
	fn into_response(self) -> Response {
		Response::default()
	}
}

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

// -------------------------

impl IntoResponse for Head {
	#[inline]
	fn into_response(self) -> Response {
		Response::from_parts(self, BoxedBody::new(Empty::new().map_err(Into::into)))
	}
}

impl IntoResponse for StatusCode {
	#[inline]
	fn into_response(self) -> Response {
		let mut response = Response::default();
		*response.status_mut() = self;

		response
	}
}

// -------------------------

impl IntoResponse for &'static str {
	#[inline]
	fn into_response(self) -> Response {
		Cow::<'_, str>::Borrowed(self).into_response()
	}
}

impl IntoResponse for String {
	#[inline]
	fn into_response(self) -> Response {
		Cow::<'_, str>::Owned(self).into_response()
	}
}

impl IntoResponse for Cow<'static, str> {
	#[inline]
	fn into_response(self) -> Response {
		let mut response = Full::from(self).into_response();
		response.headers_mut().insert(
			header::CONTENT_TYPE,
			HeaderValue::from_static(mime::TEXT_PLAIN_UTF_8.as_ref()),
		);

		response
	}
}

impl IntoResponse for &'static [u8] {
	#[inline]
	fn into_response(self) -> Response {
		Cow::<'_, [u8]>::Borrowed(self).into_response()
	}
}

impl IntoResponse for Vec<u8> {
	#[inline]
	fn into_response(self) -> Response {
		Cow::<'_, [u8]>::Owned(self).into_response()
	}
}

impl IntoResponse for Cow<'static, [u8]> {
	#[inline]
	fn into_response(self) -> Response {
		let mut response = Full::from(self).into_response();
		response.headers_mut().insert(
			header::CONTENT_TYPE,
			HeaderValue::from_static(mime::APPLICATION_OCTET_STREAM.as_ref()),
		);

		response
	}
}

impl IntoResponse for Bytes {
	#[inline]
	fn into_response(self) -> Response {
		let mut response = Full::from(self).into_response();
		response.headers_mut().insert(
			header::CONTENT_TYPE,
			HeaderValue::from_static(mime::APPLICATION_OCTET_STREAM.as_ref()),
		);

		response
	}
}

impl IntoResponse for Empty<Bytes> {
	#[inline]
	fn into_response(self) -> Response {
		Response::new(self.map_err(Into::into).boxed())
	}
}

impl IntoResponse for Full<Bytes> {
	#[inline]
	fn into_response(self) -> Response {
		Response::new(self.map_err(Into::into).boxed())
	}
}
