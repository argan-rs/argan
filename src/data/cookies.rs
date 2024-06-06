//! HTTP cookies.

// ----------

use argan_core::response::ResponseHeadParts;
use cookie::CookieJar as InnerCookieJar;
use http::{
	header::{COOKIE, SET_COOKIE},
	HeaderMap, HeaderValue,
};

use crate::{
	common::IntoArray,
	response::{BoxedErrorResponse, IntoResponse, IntoResponseHeadParts, Response},
};

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

pub use cookie::{Cookie, CookieBuilder, Expiration, Iter, ParseError, SameSite};

#[cfg(any(feature = "private-cookies", feature = "signed-cookies"))]
pub use cookie::{Key, KeyError};

// --------------------------------------------------------------------------------

const NO_KEY: &str = "key should have been set for the handler or a node";

// --------------------------------------------------
// Cookies

/// Cookies extracted from a request and send in a response.
#[derive(Default)]
pub struct CookieJar {
	inner: InnerCookieJar,
	#[cfg(any(feature = "private-cookies", feature = "signed-cookies"))]
	some_key: Option<Key>,
}

impl CookieJar {
	#[inline(always)]
	/// Creates a new, empty jar.
	pub fn new() -> CookieJar {
		Self {
			inner: InnerCookieJar::new(),
			#[cfg(any(feature = "private-cookies", feature = "signed-cookies"))]
			some_key: None,
		}
	}

	/// Sets the cryptographic `Key` used for *private* and *signed* cookies.
	#[cfg(any(feature = "private-cookies", feature = "signed-cookies"))]
	#[inline(always)]
	pub fn with_key<K>(mut self, key: K) -> CookieJar
	where
		K: for<'k> TryFrom<&'k [u8]> + Into<Key>,
	{
		self.some_key = Some(key.into());

		self
	}

	/// Clones the `Key`.
	#[cfg(any(feature = "private-cookies", feature = "signed-cookies"))]
	pub fn clone_key(&mut self) -> Key {
		self.some_key.as_ref().expect(NO_KEY).clone()
	}

	/// Adds the cookies into the jar.
	///
	/// ```
	/// use argan::data::cookies::{Cookie, CookieJar, Key, Plain, Private};
	///
	/// let mut jar = CookieJar::new().with_key(Key::generate());
	///
	/// let cookie = Cookie::new("some_cokie_1", "value");
	///
	/// jar.add([
	///   Plain.cookie(cookie),
	///   Plain.cookie(("some_cookie_2", "value")),
	///   Private.cookie(("some_private_cookie", "value")),
	/// ]);
	/// ```
	///
	/// # Panics
	/// - when addding a *private* or *signed* `Cookie` if the jar wasn't created with a `Key`.
	pub fn add<C, const N: usize>(&mut self, cookies: C)
	where
		C: IntoArray<CookieKind, N>,
	{
		let cookies = cookies.into_array();
		for cookie in cookies {
			use CookieKind::*;

			match cookie {
				Plain(cookie) => self.inner.add(cookie),
				#[cfg(feature = "private-cookies")]
				Private(cookie) => self
					.inner
					.private_mut(self.some_key.as_ref().expect(NO_KEY))
					.add(cookie),
				#[cfg(feature = "signed-cookies")]
				Signed(cookie) => self
					.inner
					.signed_mut(self.some_key.as_ref().expect(NO_KEY))
					.add(cookie),
			}
		}
	}

	/// If exists, returns the *plain* `Cookie` with the given `name`. Otherwise, `None` is returned.
	#[inline(always)]
	pub fn plain_cookie<S: AsRef<str>>(&self, name: S) -> Option<&Cookie<'static>> {
		self.inner.get(name.as_ref())
	}

	/// If exists, retrieves the *private* `Cookie` with the given `name`, authenticates and decrypts
	/// it with the jar's `Key`, and returns it as a *plain* `Cookie`. If the `Cookie` doesn't exist
	/// or the authentication and decryption fail, `None` is returned.
	///
	/// # Panics
	/// - if the jar wasn't created with a `Key`.
	#[cfg(feature = "private-cookies")]
	#[inline(always)]
	pub fn private_cookie<S: AsRef<str>>(&self, name: S) -> Option<Cookie<'static>> {
		self
			.inner
			.private(self.some_key.as_ref().expect(NO_KEY))
			.get(name.as_ref())
	}

	/// If exists, retrieves the *signed* `Cookie` with the given `name`, verifies its
	/// authenticity and integrity with the jar's `Key`, and returns it as a *plain* `Cookie`.
	/// If the `Cookie` doesn't exist or the verification fails, `None` is returned.
	///
	/// # Panics
	/// - if the jar wasn't created with a `Key`.
	#[cfg(feature = "signed-cookies")]
	#[inline(always)]
	pub fn signed_cookie<S: AsRef<str>>(&self, name: S) -> Option<Cookie<'static>> {
		self
			.inner
			.signed(self.some_key.as_ref().expect(NO_KEY))
			.get(name.as_ref())
	}

	/// Removes the cookies from the jar.
	///
	/// If the cookies in the jar were extracted from a request, removing a cookie creates
	/// a *removal* cookie (a cookie that has the same name as the original but has an empty
	/// value, a max-age of 0, and an expiration date in the past). If the original cookie
	/// was created with a `path` and/or `domain`, the cookie passed to `remove()` must have
	/// the same `path` and/or `domain` to properly create a *removal* cookie.
	///
	/// ```
	/// use argan::{
	///   resource::Resource,
	///   request::RequestHead,
	///   handler::HandlerSetter,
	///   http::Method,
	///   data::cookies::{Cookie, CookieJar, Key, Plain, Private},
	/// };
	///
	/// async fn handler(mut request_head: RequestHead) -> CookieJar {
	///   let mut jar = request_head.cookies();
	///
	///   let cookie = Cookie::build("some_cookie").path("/resource").domain("example.com");
	///
	///   // To remove a cookie, we only need to pass it as a plain cookie. So using
	///   // `Private` and `Signed` is optional, but they can be used to document the
	///   // cookie's type for the reader.
	///   jar.remove([
	///     Plain.cookie(cookie),
	///     Plain.cookie("some_cookie_2"),
	///     Private.cookie("some_private_cookie"),
	///   ]);
	///
	///   jar
	/// }
	///
	/// let mut resource = Resource::new("/");
	/// resource.set_handler_for(Method::GET.to(handler));
	/// ```
	pub fn remove<C, const N: usize>(&mut self, cookies: C)
	where
		C: IntoArray<CookieKind, N>,
	{
		let cookies = cookies.into_array();
		for cookie in cookies {
			use CookieKind::*;

			#[allow(clippy::infallible_destructuring_match)]
			let cookie = match cookie {
				Plain(cookie) => cookie,
				#[cfg(feature = "private-cookies")]
				Private(cookie) => cookie,
				#[cfg(feature = "signed-cookies")]
				Signed(cookie) => cookie,
			};

			self.inner.remove(cookie);
		}
	}

	/// Converts the `CookieJar` into `PrivateCookieJar`. Can be used when working only with
	/// private cookies.
	///
	/// # Panics
	/// - if the `CookieJar` wasn't created with a `Key`.
	#[cfg(feature = "private-cookies")]
	#[inline(always)]
	pub fn into_private_jar(self) -> PrivateCookieJar {
		PrivateCookieJar {
			inner: self.inner,
			key: self.some_key.expect(NO_KEY),
		}
	}

	/// Converts the `CookieJar` into `SignedCookieJar`. Can be used when working only with
	/// signed cookies.
	///
	/// # Panics
	/// - if the `CookieJar` wasn't created with a `Key`.
	#[cfg(feature = "signed-cookies")]
	#[inline(always)]
	pub fn into_signed_jar(self) -> SignedCookieJar {
		SignedCookieJar {
			inner: self.inner,
			key: self.some_key.expect(NO_KEY),
		}
	}

	/// Returns an iterator over all of the cookies in the jar.
	#[inline(always)]
	pub fn iter(&self) -> Iter<'_> {
		self.inner.iter()
	}
}

pub(crate) fn cookies_from_request(
	head: &HeaderMap,
	#[cfg(any(feature = "private-cookies", feature = "signed-cookies"))] some_key: Option<Key>,
) -> CookieJar {
	let cookie_jar = head
		.get(COOKIE)
		.and_then(|value| {
			value
				.to_str()
				.ok()
				.map(Cookie::split_parse_encoded)
				.map(|cookies| {
					cookies.fold(CookieJar::new(), |mut jar, result| {
						if let Ok(cookie) = result {
							jar.inner.add_original(cookie.into_owned());
						}

						jar
					})
				})
		})
		.unwrap_or_default();

	#[cfg(any(feature = "private-cookies", feature = "signed-cookies"))]
	if let Some(key) = some_key {
		return cookie_jar.with_key(key);
	}

	cookie_jar
}

// -------------------------

impl IntoResponseHeadParts for CookieJar {
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

// --------------------------------------------------
// PrivateCookieJar

/// A private cookie jar that automatically encrypts the added cookies and decrypts
/// the retrieved cookies.
#[cfg(feature = "private-cookies")]
pub struct PrivateCookieJar {
	inner: InnerCookieJar,
	key: Key,
}

#[cfg(feature = "private-cookies")]
impl PrivateCookieJar {
	/// Creates a new, empty cookie jar that treats added and retrieved cookies as private.
	pub fn new(key: Key) -> Self {
		Self {
			inner: InnerCookieJar::new(),
			key,
		}
	}

	/// If exists, retrieves the *private* `Cookie` with the given `name`, authenticates and
	/// decrypts it with the jar's `Key`, and returns it as a *plain* `Cookie`. If the cookie
	/// doesn't exist or the authentication and decryption fail, `None` is returned.
	#[inline(always)]
	pub fn get<S: AsRef<str>>(&self, name: S) -> Option<Cookie<'static>> {
		self.inner.private(&self.key).get(name.as_ref())
	}

	/// Adds the given cookie to the jar, encrypting its value.
	#[inline(always)]
	pub fn add<C: Into<Cookie<'static>>>(&mut self, cookie: C) {
		self.inner.private_mut(&self.key).add(cookie.into());
	}

	/// Removes the *private* `Cookie` from the jar.
	///
	/// The creation of the *removal* cookie is the same as for [CookieJar::remove()].
	#[inline(always)]
	pub fn remove<C: Into<Cookie<'static>>>(&mut self, cookie: C) {
		self.inner.private_mut(&self.key).remove(cookie.into());
	}

	/// Authenticates and decrypts the given cookie. If decryption succeeds, returns it
	/// as a *plain* `Cookie`. Otherwise `None` is returned.
	#[inline(always)]
	pub fn decrypt(&mut self, cookie: Cookie<'static>) -> Option<Cookie<'static>> {
		self.inner.private_mut(&self.key).decrypt(cookie)
	}

	/// Converts the `PrivateCookieJar` back to `CookieJar`.
	#[inline(always)]
	pub fn into_jar(self) -> CookieJar {
		CookieJar {
			inner: self.inner,
			some_key: Some(self.key),
		}
	}
}

// -------------------------

#[cfg(feature = "private-cookies")]
impl IntoResponseHeadParts for PrivateCookieJar {
	fn into_response_head(
		self,
		head: ResponseHeadParts,
	) -> Result<ResponseHeadParts, BoxedErrorResponse> {
		self.into_jar().into_response_head(head)
	}
}

#[cfg(feature = "private-cookies")]
impl IntoResponse for PrivateCookieJar {
	fn into_response(self) -> Response {
		let (head, body) = Response::default().into_parts();

		match self.into_response_head(head) {
			Ok(head) => Response::from_parts(head, body),
			Err(_) => unreachable!("encoded cookie must always be a valid header value"),
		}
	}
}

// --------------------------------------------------
// SignedCookieJar

/// A signed cookie jar that automatically signs the added cookies and verifies the
/// authenticity and integrity of the retrieved cookies.
#[cfg(feature = "signed-cookies")]
pub struct SignedCookieJar {
	inner: InnerCookieJar,
	key: Key,
}

#[cfg(feature = "signed-cookies")]
impl SignedCookieJar {
	/// Creates a new, empty cookie jar that treats added and retrieved cookies as signed.
	pub fn new(key: Key) -> Self {
		Self {
			inner: InnerCookieJar::new(),
			key,
		}
	}

	/// If exists, retrieves the *signed* `Cookie` with the given `name`, verifies its
	/// authenticity and integrity with the jar's `Key`, and returns it as a *plain* `Cookie`.
	/// If the cookie doesn't exist or the verification fails, `None` is returned.
	#[inline(always)]
	pub fn get<S: AsRef<str>>(&self, name: S) -> Option<Cookie<'static>> {
		self.inner.signed(&self.key).get(name.as_ref())
	}

	/// Adds the given cookie to the jar, signing its value.
	#[inline(always)]
	pub fn add<C: Into<Cookie<'static>>>(&mut self, cookie: C) {
		self.inner.signed_mut(&self.key).add(cookie.into());
	}

	/// Removes the *signed* `Cookie` from the jar.
	///
	/// The creation of the *removal* cookie is the same as for [CookieJar::remove()].
	#[inline(always)]
	pub fn remove<C: Into<Cookie<'static>>>(&mut self, cookie: C) {
		self.inner.signed_mut(&self.key).remove(cookie.into());
	}

	/// Verifies the authenticity and integrity of the given cookie. If verification succeeds,
	/// returns it as a *plain* `Cookie`. Otherwise `None` is returned.
	#[inline(always)]
	pub fn verify(&mut self, cookie: Cookie<'static>) -> Option<Cookie<'static>> {
		self.inner.signed_mut(&self.key).verify(cookie)
	}

	/// Converts the `SignedCookieJar` back to `CookieJar`.
	#[inline(always)]
	pub fn into_jar(self) -> CookieJar {
		CookieJar {
			inner: self.inner,
			some_key: Some(self.key),
		}
	}
}

// -------------------------

#[cfg(feature = "signed-cookies")]
impl IntoResponseHeadParts for SignedCookieJar {
	fn into_response_head(
		self,
		head: ResponseHeadParts,
	) -> Result<ResponseHeadParts, BoxedErrorResponse> {
		self.into_jar().into_response_head(head)
	}
}

#[cfg(feature = "signed-cookies")]
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
		#[cfg(feature = "private-cookies")]
		Private(Cookie<'static>),
		#[cfg(feature = "signed-cookies")]
		Signed(Cookie<'static>),
	}

	impl IntoArray<CookieKind, 1> for CookieKind {
		fn into_array(self) -> [CookieKind; 1] {
			[self]
		}
	}
}

use private::CookieKind;

/// A type that represents a *plain* cookie.
pub struct Plain;

impl Plain {
	/// Passes the cookie to the jar as a *plain* `Cookie`.
	#[inline(always)]
	pub fn cookie<C: Into<Cookie<'static>>>(self, cookie: C) -> CookieKind {
		CookieKind::Plain(cookie.into())
	}
}

/// A type that represents a *private* cookie.
#[cfg(feature = "private-cookies")]
pub struct Private;

#[cfg(feature = "private-cookies")]
impl Private {
	/// Passes the cookie to the jar as a *private* `Cookie`.
	#[inline(always)]
	pub fn cookie<C: Into<Cookie<'static>>>(self, cookie: C) -> CookieKind {
		CookieKind::Private(cookie.into())
	}
}

/// A type that represents a *signed* cookie.
#[cfg(feature = "signed-cookies")]
pub struct Signed;

#[cfg(feature = "signed-cookies")]
impl Signed {
	/// Passes the cookie to the jar as a *signed* `Cookie`.
	#[inline(always)]
	pub fn cookie<C: Into<Cookie<'static>>>(self, cookie: C) -> CookieKind {
		CookieKind::Signed(cookie.into())
	}
}

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

#[cfg(all(test, feature = "full"))]
mod test {
	use bytes::Bytes;
	use http::Request;
	use http_body_util::Empty;

	use super::*;

	// --------------------------------------------------------------------------------
	// --------------------------------------------------------------------------------

	#[test]
	fn cookies() {
		let request = Request::builder()
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
			Private.cookie(("key1", "value1")),
			Signed.cookie(("key2", "value2")),
			Private.cookie(("key3", "value3")),
			Signed.cookie(("key4", "value4")),
		]);

		let mut cookies_string = String::new();

		for cookie in cookies.inner.delta() {
			let cookie_string = cookie.encoded().to_string();

			cookies_string.push_str(&cookie_string);
			cookies_string.push_str("; ");
		}

		let request = Request::builder()
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
