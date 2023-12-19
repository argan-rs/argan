use std::{
	borrow::Cow,
	convert::Infallible,
	fmt::Debug,
	future::{ready, Future, Ready},
	marker::PhantomData,
	pin::{pin, Pin},
	task::{Context, Poll},
};

use cookie::{Cookie, CookieJar};
use http::{
	header::{CONTENT_TYPE, COOKIE, SET_COOKIE},
	StatusCode, Version,
};
use http_body_util::{BodyExt, Empty, Full, Limited};
use hyper::body::{Body, Bytes};
use pin_project::pin_project;
use serde::{de::DeserializeOwned, Serialize};

use crate::{
	request::{FromRequest, FromRequestHead, Head as RequestHead, Request},
	response::{Head as ResponseHead, IntoResponse, IntoResponseHead, Response},
	utils::BoxedError,
};

// ----------

pub use http::{header, Extensions, HeaderMap, HeaderName, HeaderValue};

// --------------------------------------------------

pub mod form;

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

// --------------------------------------------------
// Version

impl FromRequestHead for Version {
	type Error = Infallible;
	type Future = Ready<Result<Self, Self::Error>>;

	fn from_request_head(head: &mut RequestHead) -> Self::Future {
		ready(Ok(head.version))
	}
}

// --------------------------------------------------
// HeaderMap

impl FromRequestHead for HeaderMap {
	type Error = Infallible;
	type Future = Ready<Result<Self, Self::Error>>;

	fn from_request_head(head: &mut RequestHead) -> Self::Future {
		ready(Ok(head.headers.clone()))
	}
}

impl IntoResponseHead for HeaderMap<HeaderValue> {
	type Error = Infallible;

	#[inline]
	fn into_response_head(self, mut head: ResponseHead) -> Result<ResponseHead, Self::Error> {
		head.headers.extend(self);

		Ok(head)
	}
}

// --------------------------------------------------
// Extensions

// TODO: FromRequestHead implementation?

impl IntoResponseHead for Extensions {
	type Error = Infallible;

	#[inline]
	fn into_response_head(self, mut head: ResponseHead) -> Result<ResponseHead, Self::Error> {
		head.extensions.extend(self);

		Ok(head)
	}
}

// --------------------------------------------------
// Json

pub struct Json<T, const SIZE_LIMIT: usize = { 2 * 1024 * 1024 }>(pub T);

impl<B, T, const SIZE_LIMIT: usize> FromRequest<B> for Json<T, SIZE_LIMIT>
where
	B: Body,
	B::Error: Into<BoxedError>,
	T: DeserializeOwned,
{
	type Error = Response;
	type Future = JsonFuture<B, T, SIZE_LIMIT>;

	fn from_request(request: Request<B>) -> Self::Future {
		JsonFuture {
			request,
			_mark: PhantomData,
		}
	}
}

#[pin_project]
pub struct JsonFuture<B, T, const SIZE_LIMIT: usize> {
	#[pin]
	request: Request<B>,
	_mark: PhantomData<T>,
}

impl<B, T, const SIZE_LIMIT: usize> Future for JsonFuture<B, T, SIZE_LIMIT>
where
	B: Body,
	B::Error: Into<BoxedError>,
	T: DeserializeOwned,
{
	type Output = Result<Json<T, SIZE_LIMIT>, Response>;

	fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
		let self_projection = self.project();

		let content_type = self_projection
			.request
			.headers()
			.get(CONTENT_TYPE)
			.unwrap()
			.to_str()
			.unwrap();

		if content_type == mime::APPLICATION_JSON {
			let limited_body = Limited::new(self_projection.request, SIZE_LIMIT);
			if let Poll::Ready(result) = pin!(limited_body.collect()).poll(cx) {
				let body = result.unwrap().to_bytes();
				let value = serde_json::from_slice::<T>(&body).unwrap();

				return Poll::Ready(Ok(Json(value)));
			}

			Poll::Pending
		} else {
			Poll::Ready(Err(StatusCode::UNSUPPORTED_MEDIA_TYPE.into_response()))
		}
	}
}

impl<T> IntoResponse for Json<T>
where
	T: Serialize,
{
	fn into_response(self) -> Response {
		let mut response = serde_json::to_string(&self.0).unwrap().into_response();
		response.headers_mut().insert(
			CONTENT_TYPE,
			HeaderValue::from_static(mime::APPLICATION_JSON.as_ref()),
		);

		response
	}
}

// --------------------------------------------------
// Cookies

pub struct Cookies(CookieJar);

impl FromRequestHead for Cookies {
	type Error = Infallible;
	type Future = Ready<Result<Self, Self::Error>>;

	fn from_request_head(head: &mut RequestHead) -> Self::Future {
		let cookie_jar = head
			.headers
			.get_all(COOKIE)
			.iter()
			.filter_map(|value| value.to_str().ok())
			.flat_map(Cookie::split_parse_encoded)
			.fold(CookieJar::new(), |mut jar, result| {
				match result {
					Ok(cookie) => jar.add_original(cookie.into_owned()),
					Err(_) => todo!(),
				}

				jar
			});

		ready(Ok(Cookies(cookie_jar)))
	}
}

impl IntoResponseHead for Cookies {
	type Error = Infallible;

	fn into_response_head(self, mut head: ResponseHead) -> Result<ResponseHead, Self::Error> {
		for cookie in self.0.delta() {
			match HeaderValue::try_from(cookie.encoded().to_string()) {
				Ok(header_value) => head.headers.append(SET_COOKIE, header_value),
				Err(_) => todo!(),
			};
		}

		Ok(head)
	}
}

// --------------------------------------------------
// &'static str

impl IntoResponse for &'static str {
	#[inline]
	fn into_response(self) -> Response {
		Cow::<'_, str>::Borrowed(self).into_response()
	}
}

// --------------------------------------------------
// String

impl<B> FromRequest<B> for String
where
	B: Body,
	B::Error: Debug,
{
	type Error = Response;
	type Future = StringFuture<B>;

	fn from_request(request: Request<B>) -> Self::Future {
		StringFuture(request)
	}
}

#[pin_project]
pub struct StringFuture<B>(#[pin] Request<B>);

impl<B> Future for StringFuture<B>
where
	B: Body,
	B::Error: Debug,
{
	type Output = Result<String, Response>;

	fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
		let self_projection = self.project();

		let content_type = self_projection
			.0
			.headers()
			.get(CONTENT_TYPE)
			.unwrap()
			.to_str()
			.unwrap();

		if content_type == mime::TEXT_PLAIN_UTF_8 {
			if let Poll::Ready(result) = pin!(self_projection.0.collect()).poll(cx) {
				let body = result.unwrap().to_bytes();
				let value = String::from_utf8(body.to_vec()).unwrap();

				return Poll::Ready(Ok(value));
			}

			Poll::Pending
		} else {
			Poll::Ready(Err(StatusCode::UNSUPPORTED_MEDIA_TYPE.into_response()))
		}
	}
}

impl IntoResponse for String {
	#[inline]
	fn into_response(self) -> Response {
		Cow::<'_, str>::Owned(self).into_response()
	}
}

// --------------------------------------------------
// Cow<'static, str>

impl IntoResponse for Cow<'static, str> {
	#[inline]
	fn into_response(self) -> Response {
		let mut response = Full::from(self).into_response();
		response.headers_mut().insert(
			CONTENT_TYPE,
			HeaderValue::from_static(mime::TEXT_PLAIN_UTF_8.as_ref()),
		);

		response
	}
}

// --------------------------------------------------
// &'static [u8]

impl IntoResponse for &'static [u8] {
	#[inline]
	fn into_response(self) -> Response {
		Cow::<'_, [u8]>::Borrowed(self).into_response()
	}
}

// --------------------------------------------------
// Vec<u8>

impl<B> FromRequest<B> for Vec<u8>
where
	B: Body,
	B::Error: Debug,
{
	type Error = Response;
	type Future = VecFuture<B>;

	fn from_request(request: Request<B>) -> Self::Future {
		VecFuture(request)
	}
}

#[pin_project]
pub struct VecFuture<B>(#[pin] Request<B>);

impl<B> Future for VecFuture<B>
where
	B: Body,
	B::Error: Debug,
{
	type Output = Result<Vec<u8>, Response>;

	fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
		let self_projection = self.project();

		let content_type = self_projection
			.0
			.headers()
			.get(CONTENT_TYPE)
			.unwrap()
			.to_str()
			.unwrap();

		if content_type == mime::APPLICATION_OCTET_STREAM {
			if let Poll::Ready(result) = pin!(self_projection.0.collect()).poll(cx) {
				let value = result.unwrap().to_bytes().to_vec();

				return Poll::Ready(Ok(value));
			}

			Poll::Pending
		} else {
			Poll::Ready(Err(StatusCode::UNSUPPORTED_MEDIA_TYPE.into_response()))
		}
	}
}

impl IntoResponse for Vec<u8> {
	#[inline]
	fn into_response(self) -> Response {
		Cow::<'_, [u8]>::Owned(self).into_response()
	}
}

// --------------------------------------------------
// Cow<'static, [u8]>

impl IntoResponse for Cow<'static, [u8]> {
	#[inline]
	fn into_response(self) -> Response {
		let mut response = Full::from(self).into_response();
		response.headers_mut().insert(
			CONTENT_TYPE,
			HeaderValue::from_static(mime::APPLICATION_OCTET_STREAM.as_ref()),
		);

		response
	}
}

// --------------------------------------------------
// Bytes

impl<B> FromRequest<B> for Bytes
where
	B: Body,
	B::Error: Debug,
{
	type Error = Response;
	type Future = BytesFuture<B>;

	fn from_request(request: Request<B>) -> Self::Future {
		BytesFuture(request)
	}
}

#[pin_project]
pub struct BytesFuture<B>(#[pin] Request<B>);

impl<B> Future for BytesFuture<B>
where
	B: Body,
	B::Error: Debug,
{
	type Output = Result<Bytes, Response>;

	fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
		let self_projection = self.project();

		let content_type = self_projection
			.0
			.headers()
			.get(CONTENT_TYPE)
			.unwrap()
			.to_str()
			.unwrap();

		if content_type == mime::APPLICATION_OCTET_STREAM {
			if let Poll::Ready(result) = pin!(self_projection.0.collect()).poll(cx) {
				return Poll::Ready(Ok(result.unwrap().to_bytes()));
			}

			Poll::Pending
		} else {
			Poll::Ready(Err(StatusCode::UNSUPPORTED_MEDIA_TYPE.into_response()))
		}
	}
}

impl IntoResponse for Bytes {
	#[inline]
	fn into_response(self) -> Response {
		let mut response = Full::from(self).into_response();
		response.headers_mut().insert(
			CONTENT_TYPE,
			HeaderValue::from_static(mime::APPLICATION_OCTET_STREAM.as_ref()),
		);

		response
	}
}

// --------------------------------------------------
// Empty<Bytes>

impl IntoResponse for Empty<Bytes> {
	#[inline]
	fn into_response(self) -> Response {
		Response::new(self.map_err(Into::into).boxed())
	}
}

// --------------------------------------------------
// Full<Bytes>

impl IntoResponse for Full<Bytes> {
	#[inline]
	fn into_response(self) -> Response {
		Response::new(self.map_err(Into::into).boxed())
	}
}

// --------------------------------------------------
