use std::{
	borrow::{Borrow, BorrowMut, Cow},
	convert::Infallible,
	marker::PhantomData,
};

use cookie::{prefix::Prefix, CookieJar as InnerCookieJar};
use http::{
	header::{COOKIE, SET_COOKIE},
	HeaderValue,
};

use crate::{
	common::{IntoArray, SCOPE_VALIDITY},
	request::{FromRequestHead, RequestHead},
	response::{IntoResponseHead, ResponseHead},
};

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

pub use cookie::{
	prefix::{Host, Secure},
	Cookie, Iter, Key,
};

// --------------------------------------------------
// Cookies

pub struct CookieJar<K = Key> {
	inner: InnerCookieJar,
	some_key: Option<Key>,
	_key_mark: PhantomData<K>,
}

impl<K> CookieJar<K>
where
	K: for<'k> TryFrom<&'k [u8]> + Into<Key>,
{
	#[inline(always)]
	pub fn new() -> CookieJar<K> {
		Self {
			inner: InnerCookieJar::new(),
			some_key: None,
			_key_mark: PhantomData,
		}
	}

	#[inline(always)]
	pub fn with_key(mut self, key: Key) -> CookieJar<K> {
		self.some_key = Some(key);

		self
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
					.private_mut(self.some_key.as_ref().expect(SCOPE_VALIDITY))
					.add(cookie),
				Signed(cookie) => self
					.inner
					.signed_mut(self.some_key.as_ref().expect(SCOPE_VALIDITY))
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
			.private(self.some_key.as_ref().expect(SCOPE_VALIDITY))
			.get(name.as_ref())
	}

	#[inline(always)]
	pub fn signed_cookie<S: AsRef<str>>(&self, name: S) -> Option<Cookie<'static>> {
		self
			.inner
			.signed(self.some_key.as_ref().expect(SCOPE_VALIDITY))
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
			key: self.some_key.expect(SCOPE_VALIDITY),
		}
	}

	#[inline(always)]
	pub fn into_signed_jar(self) -> SignedCookieJar {
		SignedCookieJar {
			inner: self.inner,
			key: self.some_key.expect(SCOPE_VALIDITY),
		}
	}

	#[inline(always)]
	pub fn iter(&self) -> Iter<'_> {
		self.inner.iter()
	}
}

impl<K> FromRequestHead for CookieJar<K>
where
	K: for<'k> TryFrom<&'k [u8]> + Into<Key>,
{
	type Error = Infallible;

	async fn from_request_head(head: &mut RequestHead) -> Result<Self, Self::Error> {
		let cookie_jar = head
			.headers
			.get_all(COOKIE)
			.iter()
			.filter_map(|value| value.to_str().ok())
			.flat_map(Cookie::split_parse_encoded)
			.fold(CookieJar::new(), |mut jar, result| {
				match result {
					Ok(cookie) => jar.inner.add_original(cookie.into_owned()),
					Err(_) => {} // Ignored.
				}

				jar
			});

		Ok(cookie_jar)
	}
}

impl IntoResponseHead for CookieJar {
	type Error = Infallible;

	fn into_response_head(self, mut head: ResponseHead) -> Result<ResponseHead, Self::Error> {
		for cookie in self.inner.delta() {
			match HeaderValue::try_from(cookie.encoded().to_string()) {
				Ok(header_value) => head.headers.append(SET_COOKIE, header_value),
				Err(_) => unreachable!(),
			};
		}

		Ok(head)
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
			_key_mark: PhantomData,
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
			_key_mark: PhantomData,
		}
	}
}

// -------------------------

pub enum CookieKind {
	Plain(Cookie<'static>),
	Private(Cookie<'static>),
	Signed(Cookie<'static>),
}

#[inline(always)]
pub fn plain<C: Into<Cookie<'static>>>(cookie: C) -> CookieKind {
	CookieKind::Plain(cookie.into())
}

#[inline(always)]
pub fn private<C: Into<Cookie<'static>>>(cookie: C) -> CookieKind {
	CookieKind::Private(cookie.into())
}

#[inline(always)]
pub fn signed<C: Into<Cookie<'static>>>(cookie: C) -> CookieKind {
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
