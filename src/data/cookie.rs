use std::{
	borrow::{Borrow, BorrowMut, Cow},
	convert::Infallible,
	default,
	future::Future,
	marker::PhantomData,
};

use argan_core::{request::RequestHeadParts, response::ResponseHeadParts, IntoArray};
use bytes::Bytes;
use cookie::{prefix::Prefix, CookieJar as InnerCookieJar};
use http::{
	header::{COOKIE, SET_COOKIE},
	HeaderMap, HeaderValue,
};

use crate::{
	common::SCOPE_VALIDITY,
	handler::Args,
	request::RequestHead,
	response::{BoxedErrorResponse, IntoResponse, IntoResponseHead, Response},
	routing::RoutingState,
};

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

pub use cookie::{
	prefix::{Host as PrefixHost, Secure as PrefixSecure},
	Cookie, CookieBuilder, Expiration, Iter, Key, KeyError, ParseError, SameSite,
};

// --------------------------------------------------------------------------------

const NO_KEY: &'static str = "key should have been set";

// --------------------------------------------------
// Cookies

#[derive(Default)]
pub struct CookieJar {
	inner: InnerCookieJar,
	some_key: Option<Key>,
}

impl CookieJar {
	#[inline(always)]
	pub fn new() -> CookieJar {
		Self {
			inner: InnerCookieJar::new(),
			some_key: None,
		}
	}

	#[inline(always)]
	pub fn with_key<K>(mut self, key: K) -> CookieJar
	where
		K: for<'k> TryFrom<&'k [u8]> + Into<Key>,
	{
		self.some_key = Some(key.into());

		self
	}

	pub fn clone_key(&mut self) -> Key {
		self.some_key.as_ref().expect(NO_KEY).clone()
	}

	pub fn add<C, const N: usize>(&mut self, cookies: C)
	where
		C: IntoArray<CookieKind, N>,
	{
		let cookies = cookies.into_array();
		for cookie in cookies {
			use CookieKind::*;

			match cookie {
				Plain(cookie) => self.inner.add(cookie),
				Private(cookie) => self
					.inner
					.private_mut(self.some_key.as_ref().expect(NO_KEY))
					.add(cookie),
				Signed(cookie) => self
					.inner
					.signed_mut(self.some_key.as_ref().expect(NO_KEY))
					.add(cookie),
			}
		}
	}

	#[inline(always)]
	pub fn plain_cookie<S: AsRef<str>>(&self, name: S) -> Option<&Cookie<'static>> {
		self.inner.get(name.as_ref())
	}

	#[inline(always)]
	pub fn private_cookie<S: AsRef<str>>(&self, name: S) -> Option<Cookie<'static>> {
		self
			.inner
			.private(self.some_key.as_ref().expect(NO_KEY))
			.get(name.as_ref())
	}

	#[inline(always)]
	pub fn signed_cookie<S: AsRef<str>>(&self, name: S) -> Option<Cookie<'static>> {
		self
			.inner
			.signed(self.some_key.as_ref().expect(NO_KEY))
			.get(name.as_ref())
	}

	pub fn remove<C, const N: usize>(&mut self, cookies: C)
	where
		C: IntoArray<CookieKind, N>,
	{
		let cookies = cookies.into_array();
		for cookie in cookies {
			use CookieKind::*;

			let cookie = match cookie {
				Plain(cookie) => cookie,
				Private(cookie) => cookie,
				Signed(cookie) => cookie,
			};

			self.inner.remove(cookie);
		}
	}

	#[inline(always)]
	pub fn into_private_jar(self) -> PrivateCookieJar {
		PrivateCookieJar {
			inner: self.inner,
			key: self.some_key.expect(NO_KEY),
		}
	}

	#[inline(always)]
	pub fn into_signed_jar(self) -> SignedCookieJar {
		SignedCookieJar {
			inner: self.inner,
			key: self.some_key.expect(NO_KEY),
		}
	}

	// #[inline(always)]
	// pub fn iter(&self) -> Iter<'_> {
	// 	self.inner.iter()
	// }
}

// impl<K> FromMutRequestHead for CookieJar<K>
// where
// 	K: for<'k> TryFrom<&'k [u8]> + Into<Key>,
// {
// 	type Error = Infallible;
//
// 	async fn from_request_head(head: &mut RequestHead) -> Result<Self, Self::Error> {
// 		let cookie_jar = head
// 			.headers
// 			.get_all(COOKIE)
// 			.iter()
// 			.filter_map(|value| value.to_str().ok())
// 			.flat_map(Cookie::split_parse_encoded)
// 			.fold(CookieJar::new(), |mut jar, result| {
// 				match result {
// 					Ok(cookie) => jar.inner.add_original(cookie.into_owned()),
// 					Err(_) => {} // Ignored.
// 				}
//
// 				jar
// 			});
//
// 		Ok(cookie_jar)
// 	}
// }

// impl<'r, B> FromRequestRef<'r, B> for CookieJar
// where
// 	B: Sync,
// {
// 	type Error = Infallible;
//
// 	async fn from_request_ref(request: &'r Request<B>) -> Result<Self, Self::Error> {
// 		Ok(cookies_from_request(request, None))
// 	}
// }

// impl<B> FromRequestBody<B> for CookieJar
// where
// 	B: Send,
// {
// 	type Error = Infallible;
//
// 	async fn from_request_body(mut head_parts: RequestHeadParts, body: B) -> (RequestHeadParts, Result<Self, Self::Error>) {
// 		Ok(cookies_from_request(&head_parts, None))
// 	}
// }

pub(crate) fn cookies_from_request(head: &HeaderMap, some_key: Option<Key>) -> CookieJar {
	let cookie_jar = head
		.get(COOKIE)
		.and_then(|value| {
			value
				.to_str()
				.ok()
				.map(Cookie::split_parse_encoded)
				.map(|cookies| {
					cookies.fold(CookieJar::new(), |mut jar, result| {
						match result {
							Ok(cookie) => jar.inner.add_original(cookie.into_owned()),
							Err(_) => {} // Ignored.
						}

						jar
					})
				})
		})
		.unwrap_or_default();

	if some_key.is_some() {
		return cookie_jar.with_key(some_key.expect(SCOPE_VALIDITY));
	}

	cookie_jar
}

// -------------------------

impl IntoResponseHead for CookieJar {
	fn into_response_head(
		self,
		mut head: ResponseHeadParts,
	) -> Result<ResponseHeadParts, BoxedErrorResponse> {
		for cookie in self.inner.delta() {
			match HeaderValue::try_from(cookie.encoded().to_string()) {
				Ok(header_value) => head.headers.append(SET_COOKIE, header_value),
				Err(_) => unreachable!("encoded cookie must always be a valid header value"),
			};
		}

		Ok(head)
	}
}

impl IntoResponse for CookieJar {
	fn into_response(self) -> Response {
		let (head, body) = Response::default().into_parts();

		match self.into_response_head(head) {
			Ok(head) => Response::from_parts(head, body),
			Err(_) => unreachable!("encoded cookie must always be a valid header value"),
		}
	}
}

// -------------------------

pub struct PrivateCookieJar {
	inner: InnerCookieJar,
	key: Key,
}

impl PrivateCookieJar {
	#[inline(always)]
	pub fn get<S: AsRef<str>>(&self, name: S) -> Option<Cookie<'static>> {
		self.inner.private(&self.key).get(name.as_ref())
	}

	#[inline(always)]
	pub fn add<C: Into<Cookie<'static>>>(&mut self, cookie: C) {
		self.inner.private_mut(&self.key).add(cookie.into());
	}

	#[inline(always)]
	pub fn remove<C: Into<Cookie<'static>>>(&mut self, cookie: C) {
		self.inner.private_mut(&self.key).remove(cookie.into());
	}

	#[inline(always)]
	pub fn decrypt(&mut self, cookie: Cookie<'static>) -> Option<Cookie<'static>> {
		self.inner.private_mut(&self.key).decrypt(cookie)
	}

	#[inline(always)]
	pub fn into_jar(self) -> CookieJar {
		CookieJar {
			inner: self.inner,
			some_key: Some(self.key),
		}
	}
}

// impl<K> FromMutRequestHead for PrivateCookieJar<K>
// where
// 	K: for<'k> TryFrom<&'k [u8]> + Into<Key>,
// {
// 	type Error = Infallible;
//
// 	async fn from_request_head(head: &mut RequestHead) -> Result<Self, Self::Error> {
// 		<CookieJar<K> as FromMutRequestHead>::from_request_head(head)
// 			.await
// 			.map(|jar| jar.into_private_jar())
// 	}
// }

// impl<'r, B> FromRequestRef<'r, B> for PrivateCookieJar
// where
// 	B: Sync,
// {
// 	type Error = Infallible;
//
// 	async fn from_request_ref(request: &'r Request<B>) -> Result<Self, Self::Error> {
// 		<CookieJar as FromRequestRef<'r, B>>::from_request_ref(request)
// 			.await
// 			.map(|jar| jar.into_private_jar())
// 	}
// }

// impl<B> FromRequest<B> for PrivateCookieJar
// where
// 	B: Send,
// {
// 	type Error = Infallible;
//
// 	async fn from_request(request: Request<B>) -> Result<Self, Self::Error> {
// 		<CookieJar as FromRequest<B>>::from_request(request)
// 			.await
// 			.map(|jar| jar.into_private_jar())
// 	}
// }

impl IntoResponseHead for PrivateCookieJar {
	fn into_response_head(
		self,
		mut head: ResponseHeadParts,
	) -> Result<ResponseHeadParts, BoxedErrorResponse> {
		self.into_jar().into_response_head(head)
	}
}

impl IntoResponse for PrivateCookieJar {
	fn into_response(self) -> Response {
		let (head, body) = Response::default().into_parts();

		match self.into_response_head(head) {
			Ok(head) => Response::from_parts(head, body),
			Err(_) => unreachable!("encoded cookie must always be a valid header value"),
		}
	}
}

// -------------------------

pub struct SignedCookieJar {
	inner: InnerCookieJar,
	key: Key,
}

impl SignedCookieJar {
	#[inline(always)]
	pub fn get<S: AsRef<str>>(&self, name: S) -> Option<Cookie<'static>> {
		self.inner.signed(&self.key).get(name.as_ref())
	}

	#[inline(always)]
	pub fn add<C: Into<Cookie<'static>>>(&mut self, cookie: C) {
		self.inner.signed_mut(&self.key).add(cookie.into());
	}

	#[inline(always)]
	pub fn remove<C: Into<Cookie<'static>>>(&mut self, cookie: C) {
		self.inner.signed_mut(&self.key).remove(cookie.into());
	}

	#[inline(always)]
	pub fn verify(&mut self, cookie: Cookie<'static>) -> Option<Cookie<'static>> {
		self.inner.signed_mut(&self.key).verify(cookie)
	}

	#[inline(always)]
	pub fn into_jar(self) -> CookieJar {
		CookieJar {
			inner: self.inner,
			some_key: Some(self.key),
		}
	}
}

// impl<K> FromMutRequestHead for SignedCookieJar<K>
// where
// 	K: for<'k> TryFrom<&'k [u8]> + Into<Key>,
// {
// 	type Error = Infallible;
//
// 	async fn from_request_head(head: &mut RequestHead) -> Result<Self, Self::Error> {
// 		<CookieJar<K> as FromMutRequestHead>::from_request_head(head)
// 			.await
// 			.map(|jar| jar.into_signed_jar())
// 	}
// }

// impl<'r, B> FromRequestRef<'r, B> for SignedCookieJar
// where
// 	B: Sync,
// {
// 	type Error = Infallible;
//
// 	async fn from_request_ref(request: &'r Request<B>) -> Result<Self, Self::Error> {
// 		<CookieJar as FromRequestRef<'r, B>>::from_request_ref(request)
// 			.await
// 			.map(|jar| jar.into_signed_jar())
// 	}
// }

// impl<B> FromRequest<B> for SignedCookieJar
// where
// 	B: Send,
// {
// 	type Error = Infallible;
//
// 	async fn from_request(request: Request<B>) -> Result<Self, Self::Error> {
// 		<CookieJar as FromRequest<B>>::from_request(request)
// 			.await
// 			.map(|jar| jar.into_signed_jar())
// 	}
// }

impl IntoResponseHead for SignedCookieJar {
	fn into_response_head(
		self,
		mut head: ResponseHeadParts,
	) -> Result<ResponseHeadParts, BoxedErrorResponse> {
		self.into_jar().into_response_head(head)
	}
}

impl IntoResponse for SignedCookieJar {
	fn into_response(self) -> Response {
		let (head, body) = Response::default().into_parts();

		match self.into_response_head(head) {
			Ok(head) => Response::from_parts(head, body),
			Err(_) => unreachable!("encoded cookie must always be a valid header value"),
		}
	}
}

// -------------------------

mod private {
	use super::*;

	pub enum CookieKind {
		Plain(Cookie<'static>),
		Private(Cookie<'static>),
		Signed(Cookie<'static>),
	}

	impl IntoArray<CookieKind, 1> for CookieKind {
		fn into_array(self) -> [CookieKind; 1] {
			[self]
		}
	}
}

use private::CookieKind;

use super::header::HeaderMapExt;

#[inline(always)]
pub fn _plain<C: Into<Cookie<'static>>>(cookie: C) -> CookieKind {
	CookieKind::Plain(cookie.into())
}

#[inline(always)]
pub fn _private<C: Into<Cookie<'static>>>(cookie: C) -> CookieKind {
	CookieKind::Private(cookie.into())
}

#[inline(always)]
pub fn _signed<C: Into<Cookie<'static>>>(cookie: C) -> CookieKind {
	CookieKind::Signed(cookie.into())
}

// -------------------------

#[inline(always)]
pub fn prefixed_name<P, S>(_: P, name: S) -> String
where
	P: Prefix,
	S: AsRef<str>,
{
	format!("{}{}", P::PREFIX, name.as_ref())
}

pub fn prefix<P, C>(_p: P, cookie: C) -> Cookie<'static>
where
	P: Prefix,
	C: Into<Cookie<'static>>,
{
	let mut cookie = cookie.into();
	let name = prefixed_name(_p, cookie.name());
	cookie.set_name(name);

	<P as Prefix>::conform(cookie)
}

pub fn strip_prefix<P, C>(_: P, mut cookie: Cookie<'static>) -> Cookie<'static>
where
	P: Prefix,
{
	if let Some(name) = cookie
		.name()
		.strip_prefix(P::PREFIX)
		.map(|name| name.to_string())
	{
		cookie.set_name(name);
	}

	cookie
}

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

#[cfg(test)]
mod test {
	use http::Request;
	use http_body_util::Empty;

	use super::*;

	// --------------------------------------------------------------------------------
	// --------------------------------------------------------------------------------

	#[test]
	fn cookies() {
		let mut request = Request::builder()
			.uri("/")
			.header("Cookie", "key1=value1; key2=value2")
			.body(Empty::<Bytes>::default())
			.unwrap();

		let cookies = super::cookies_from_request(request.headers(), None);
		assert_eq!(cookies.inner.iter().count(), 2);
		assert_eq!(cookies.plain_cookie("key1").unwrap().value(), "value1");
		assert_eq!(cookies.plain_cookie("key2").unwrap().value(), "value2");

		// --------------------------------------------------

		let key = Key::generate();

		let mut cookies = CookieJar::new().with_key(key);
		cookies.add([
			_private(("key1", "value1")),
			_signed(("key2", "value2")),
			_private(("key3", "value3")),
			_signed(("key4", "value4")),
		]);

		let mut cookies_string = String::new();

		for cookie in cookies.inner.delta() {
			let cookie_string = cookie.encoded().to_string();

			cookies_string.push_str(&cookie_string);
			cookies_string.push_str("; ");
		}

		let mut request = Request::builder()
			.uri("/")
			.header("Cookie", cookies_string)
			.body(Empty::<Bytes>::default())
			.unwrap();

		// let mut head = request.into_parts().0.into();
		// head = head.with_cookie_key(cookies.clone_key());

		let cookies = super::cookies_from_request(request.headers(), Some(cookies.clone_key()));
		assert_eq!(cookies.inner.iter().count(), 4);

		assert_eq!(cookies.private_cookie("key1").unwrap().value(), "value1");
		assert_eq!(cookies.signed_cookie("key2").unwrap().value(), "value2");
		assert_eq!(cookies.private_cookie("key3").unwrap().value(), "value3");
		assert_eq!(cookies.signed_cookie("key4").unwrap().value(), "value4");
	}
}
