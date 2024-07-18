//! HTTP request types.

// ----------

use std::{convert::Infallible, fmt::Debug, future::Future};

#[cfg(feature = "peer-addr")]
use std::net::SocketAddr;

use argan_core::{
	body::{Body, HttpBody},
	BoxedError,
};
use futures_util::FutureExt;
use http::{Extensions, HeaderMap, HeaderValue, Method, StatusCode, Uri, Version};
use serde::Deserialize;

use crate::{
	common::header_utils::{host_header_value, HostHeaderError},
	handler::Args,
	pattern::{self, ParamsList},
	response::{BoxedErrorResponse, IntoResponse, Response},
};

#[cfg(feature = "cookies")]
use crate::data::cookies::{cookies_from_request, CookieJar};

// ----------

pub use argan_core::request::*;

// ----------

pub(crate) mod routing;
use routing::RoutingState;

#[cfg(feature = "websockets")]
pub mod websocket;

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

// --------------------------------------------------
// RequestContext

/// A [`Handler`](crate::handler::Handler) parameter that carries the request data.
pub struct RequestContext<B = Body> {
	#[cfg(feature = "peer-addr")]
	peer_addr: SocketAddr,

	request: Request<B>,
	routing_state: RoutingState,
	properties: RequestContextProperties,
}

impl<B> RequestContext<B> {
	#[inline(always)]
	pub(crate) fn new(
		#[cfg(feature = "peer-addr")] peer_addr: SocketAddr,
		request: Request<B>,
		routing_state: RoutingState,
		properties: RequestContextProperties,
	) -> Self {
		Self {
			#[cfg(feature = "peer-addr")]
			peer_addr,

			request,
			routing_state,
			properties,
		}
	}

	#[inline(always)]
	pub(crate) fn clone_valid_properties_from(
		&mut self,
		context_properties: &RequestContextProperties,
	) {
		self
			.properties
			.clone_valid_properties_from(context_properties);
	}

	/// Returns a reference to the request method.
	#[inline(always)]
	pub fn method_ref(&self) -> &Method {
		self.request.method()
	}

	/// Returns a reference to the request URI.
	#[inline(always)]
	pub fn uri_ref(&self) -> &Uri {
		self.request.uri()
	}

	/// Returns the version of HTTP that's being used for comunication.
	#[inline(always)]
	pub fn version(&self) -> Version {
		self.request.version()
	}

	/// Returns a reference to the map of request headers.
	#[inline(always)]
	pub fn headers_ref(&self) -> &HeaderMap<HeaderValue> {
		self.request.headers()
	}

	/// Returns a reference to the request extensions.
	#[inline(always)]
	pub fn extensions_ref(&self) -> &Extensions {
		self.request.extensions()
	}

	// ----------

	/// Returns a mutable reference to the `Request`
	#[inline(always)]
	pub fn request_mut(&mut self) -> &mut Request<B> {
		&mut self.request
	}

	// ----------

	/// Returns the peer address.
	#[cfg(feature = "peer-addr")]
	pub fn peer_addr(&self) -> &SocketAddr {
		&self.peer_addr
	}

	/// Returns the available cookie `Key`.
	///
	/// The key may come from a handler, if the handler was provided with a key.
	/// Otherwise, it comes from the last resource in the path that was provided
	/// with a key or from a `Router`. If no key is available, `None` is returned.
	#[cfg(any(feature = "private-cookies", feature = "signed-cookies"))]
	#[inline(always)]
	pub fn cookie_key(&self) -> Option<cookie::Key> {
		self.properties.clone_cookie_key()
	}

	/// Returns the request cookies.
	#[cfg(feature = "cookies")]
	pub fn cookies(&self) -> CookieJar {
		cookies_from_request(
			self.headers_ref(),
			#[cfg(any(feature = "private-cookies", feature = "signed-cookies"))]
			self.properties.clone_cookie_key(),
		)
	}

	/// Returns the path params deserialized as type `T`. `T` must implement the
	/// [`serde::Deserialize`] trait.
	#[inline]
	pub fn path_params_as<'r, T>(&'r self) -> Result<T, PathParamsError>
	where
		T: Deserialize<'r>,
	{
		let mut from_params_list = self.routing_state.uri_params.deserializer();

		T::deserialize(&mut from_params_list).map_err(Into::into)
	}

	/// Returns the query params deserialized as type `T`. `T` must implement the
	/// [`serde::Deserialize`] trait.
	#[cfg(feature = "query-params")]
	#[inline]
	pub fn query_params_as<'r, T>(&'r self) -> Result<T, QueryParamsError>
	where
		T: Deserialize<'r>,
	{
		let query_string = self
			.request
			.uri()
			.query()
			.ok_or(QueryParamsError::NoDataIsAvailable)?;

		serde_urlencoded::from_str::<T>(query_string).map_err(QueryParamsError::InvalidData)
	}

	/// Returns the remaining segments of the request's path without the preceding slash `/`.
	///
	/// As the request passes through the tree of resources that match the path segments of
	/// its target URI, this method can be used to get the remaining path segments from the
	/// middleware of these resources.
	#[inline(always)]
	pub fn subtree_path_segments(&self) -> &str {
		self
			.routing_state
			.route_traversal
			.remaining_segments(self.request.uri().path())
	}

	/// Consumes the `RequestContext`, returning the request's head and body.
	pub async fn into_head_and_body(self) -> (RequestHead, B)
	where
		B: HttpBody,
		B::Error: Into<BoxedError>,
	{
		let (head_parts, body) = self.request.into_parts();

		#[cfg(not(feature = "peer-addr"))]
		let request_head = RequestHead::new(head_parts, self.routing_state, self.properties);

		#[cfg(feature = "peer-addr")]
		let request_head = RequestHead::new(
			self.peer_addr,
			head_parts,
			self.routing_state,
			self.properties,
		);

		(request_head, body)
	}

	/// Consumes the `RequestContext`, extracting the `RequestHead` and type `T`.
	///
	/// `T` must implement the `FromRequest` trait.
	pub async fn extract<T>(self) -> (RequestHead, Result<T, T::Error>)
	where
		T: FromRequest<B>,
	{
		let (mut head_parts, body) = self.request.into_parts();
		let result = T::from_request(&mut head_parts, body).await;

		#[cfg(not(feature = "peer-addr"))]
		let request_head = RequestHead::new(head_parts, self.routing_state, self.properties);

		#[cfg(feature = "peer-addr")]
		let request_head = RequestHead::new(
			self.peer_addr,
			head_parts,
			self.routing_state,
			self.properties,
		);

		(request_head, result)
	}

	/// Calls the given function to map the request body and returns the `RequestContext`
	/// with the mapped body.
	pub fn map<Func, NewB>(self, func: Func) -> RequestContext<NewB>
	where
		Func: FnOnce(B) -> NewB,
	{
		let RequestContext {
			#[cfg(feature = "peer-addr")]
			peer_addr,

			request,
			routing_state,
			properties,
		} = self;

		let (head, body) = request.into_parts();

		let new_body = func(body);
		let request = Request::from_parts(head, new_body);

		RequestContext {
			#[cfg(feature = "peer-addr")]
			peer_addr,

			request,
			routing_state,
			properties,
		}
	}
}

// Crate private methods.
impl<B> RequestContext<B> {
	#[inline(always)]
	pub(crate) fn path_ends_with_slash(&self) -> bool {
		self
			.routing_state
			.route_traversal
			.ends_with_slash(self.request.uri().path())
	}

	#[inline(always)]
	pub(crate) fn routing_has_remaining_segments(&self) -> bool {
		self
			.routing_state
			.route_traversal
			.has_remaining_segments(self.request.uri().path())
	}

	#[inline(always)]
	pub(crate) fn routing_next_segment_index(&self) -> usize {
		self.routing_state.route_traversal.next_segment_index()
	}

	#[inline(always)]
	pub(crate) fn routing_host_and_uri_params_mut(
		&mut self,
	) -> Result<(&str, &mut ParamsList), HostHeaderError> {
		let host = host_header_value(&self.request)?;

		Ok((host, &mut self.routing_state.uri_params))
	}

	#[inline(always)]
	pub(crate) fn routing_next_segment_and_uri_params_mut(
		&mut self,
	) -> Option<(&str, &mut ParamsList)> {
		let (next_segment, _) = self
			.routing_state
			.route_traversal
			.next_segment(self.request.uri().path())?;

		Some((next_segment, &mut self.routing_state.uri_params))
	}

	#[inline(always)]
	pub(crate) fn routing_revert_to_segment(&mut self, segment_index: usize) {
		self
			.routing_state
			.route_traversal
			.revert_to_segment(segment_index);
	}

	#[inline(always)]
	pub(crate) fn note_subtree_handler(&mut self) {
		self.routing_state.subtree_handler_exists = true;
	}

	#[inline(always)]
	pub(crate) fn noted_subtree_handler(&self) -> bool {
		self.routing_state.subtree_handler_exists
	}

	#[cfg(not(feature = "peer-addr"))]
	#[inline(always)]
	pub(crate) fn into_parts(self) -> (Request<B>, RoutingState, RequestContextProperties) {
		(self.request, self.routing_state, self.properties)
	}

	#[cfg(feature = "peer-addr")]
	#[inline(always)]
	pub(crate) fn into_parts(
		self,
	) -> (
		SocketAddr,
		Request<B>,
		RoutingState,
		RequestContextProperties,
	) {
		(
			self.peer_addr,
			self.request,
			self.routing_state,
			self.properties,
		)
	}
}

// --------------------------------------------------
// ExtractorGuard

/// A trait for request handler guards.
pub trait ExtractorGuard<B = Body, Ext: Clone = ()>: Sized {
	type Error: Into<BoxedErrorResponse>;

	fn from_request_context_and_args(
		request_context: &mut RequestContext<B>,
		args: &Args<'static, Ext>,
	) -> impl Future<Output = Result<Self, Self::Error>> + Send;
}

// ----------

impl<B, Ext, T, E> ExtractorGuard<B, Ext> for Result<T, E>
where
	Ext: Clone,
	T: ExtractorGuard<B, Ext, Error = E>,
{
	type Error = Infallible;

	fn from_request_context_and_args(
		request_context: &mut RequestContext<B>,
		args: &Args<'static, Ext>,
	) -> impl Future<Output = Result<Self, Self::Error>> + Send {
		T::from_request_context_and_args(request_context, args).map(Ok)
	}
}

// ----------

impl<B, Ext, T> ExtractorGuard<B, Ext> for Option<T>
where
	Ext: Clone,
	T: ExtractorGuard<B, Ext>,
{
	type Error = Infallible;

	fn from_request_context_and_args(
		request_context: &mut RequestContext<B>,
		args: &Args<'static, Ext>,
	) -> impl Future<Output = Result<Self, Self::Error>> + Send {
		T::from_request_context_and_args(request_context, args).map(|result| Ok(result.ok()))
	}
}

// --------------------------------------------------
// RequestHeead

/// A type of request's head part.
pub struct RequestHead {
	#[cfg(feature = "peer-addr")]
	peer_addr: SocketAddr,

	method: Method,
	uri: Uri,
	version: Version,
	headers: HeaderMap<HeaderValue>,
	extensions: Extensions,

	routing_state: RoutingState,
	request_context_properties: RequestContextProperties,
}

impl RequestHead {
	#[inline(always)]
	pub(crate) fn new(
		#[cfg(feature = "peer-addr")] peer_addr: SocketAddr,

		head_parts: RequestHeadParts,
		routing_state: RoutingState,
		context_properties: RequestContextProperties,
	) -> Self {
		Self {
			#[cfg(feature = "peer-addr")]
			peer_addr,

			method: head_parts.method,
			uri: head_parts.uri,
			version: head_parts.version,
			headers: head_parts.headers,
			extensions: head_parts.extensions,
			routing_state,
			request_context_properties: context_properties,
		}
	}
}

impl RequestHead {
	/// Returns a reference to the request method.
	#[inline(always)]
	pub fn method_ref(&self) -> &Method {
		&self.method
	}

	/// Sets the request method.
	#[inline(always)]
	pub fn set_method(&mut self, method: Method) {
		self.method = method;
	}

	/// Returns a reference to the request URI.
	#[inline(always)]
	pub fn uri_ref(&self) -> &Uri {
		&self.uri
	}

	/// Sets the request URI.
	#[inline(always)]
	pub fn set_uri(&mut self, uri: Uri) {
		self.uri = uri;
	}

	/// Returns the version of HTTP that's being used for comunication.
	#[inline(always)]
	pub fn version(&self) -> Version {
		self.version
	}

	/// Sets the HTTP version.
	#[inline(always)]
	pub fn set_version(&mut self, version: Version) {
		self.version = version;
	}

	/// Returns a reference to the map of request headers.
	#[inline(always)]
	pub fn headers_ref(&self) -> &HeaderMap<HeaderValue> {
		&self.headers
	}

	/// Returns a mutable reference to the map of request headers.
	#[inline(always)]
	pub fn headers_mut(&mut self) -> &mut HeaderMap<HeaderValue> {
		&mut self.headers
	}

	/// Returns a reference to the request extensions.
	#[inline(always)]
	pub fn extensions_ref(&self) -> &Extensions {
		&self.extensions
	}

	/// Returns a mutable reference to the request extensions.
	#[inline(always)]
	pub fn extensions_mut(&mut self) -> &mut Extensions {
		&mut self.extensions
	}

	// ----------

	/// Returns the peer address.
	#[cfg(feature = "peer-addr")]
	pub fn peer_addr(&self) -> &SocketAddr {
		&self.peer_addr
	}

	/// Returns the available cookie `Key`.
	///
	/// The key may come from a handler, if the handler was provided with a key.
	/// Otherwise, it comes from the last resource in the path that was provided
	/// with a key or from a `Router`. If no key is available, `None` is returned.
	#[cfg(any(feature = "private-cookies", feature = "signed-cookies"))]
	#[inline(always)]
	pub fn cookie_key(&self) -> Option<cookie::Key> {
		self.request_context_properties.clone_cookie_key()
	}

	/// Returns the request cookies.
	#[cfg(feature = "cookies")]
	#[inline(always)]
	pub fn cookies(&self) -> CookieJar {
		cookies_from_request(
			&self.headers,
			#[cfg(any(feature = "private-cookies", feature = "signed-cookies"))]
			self.request_context_properties.clone_cookie_key(),
		)
	}

	/// Returns the path params deserialized as type `T`. `T` must implement the
	/// [`serde::Deserialize`] trait.
	#[inline]
	pub fn path_params_as<'r, T>(&'r self) -> Result<T, PathParamsError>
	where
		T: Deserialize<'r>,
	{
		let mut from_params_list = self.routing_state.uri_params.deserializer();

		T::deserialize(&mut from_params_list).map_err(Into::into)
	}

	/// Returns the query params deserialized as type `T`. `T` must implement the
	/// [`serde::Deserialize`] trait.
	#[cfg(feature = "query-params")]
	#[inline]
	pub fn query_params_as<'r, T>(&'r self) -> Result<T, QueryParamsError>
	where
		T: Deserialize<'r>,
	{
		let query_string = self
			.uri
			.query()
			.ok_or(QueryParamsError::NoDataIsAvailable)?;

		serde_urlencoded::from_str::<T>(query_string).map_err(QueryParamsError::InvalidData)
	}

	/// Returns the remaining segments of the request's path without the preceding slash `/`.
	///
	/// This method is intended to be used by subtree handler resources when there is no resource
	/// that matches the request's target in their subtree.
	#[inline(always)]
	pub fn subtree_path_segments(&self) -> &str {
		self
			.routing_state
			.route_traversal
			.remaining_segments(self.uri.path())
	}
}

// --------------------------------------------------
// ContextProperties

#[derive(Default, Clone)]
pub(crate) struct RequestContextProperties {
	#[cfg(any(feature = "private-cookies", feature = "signed-cookies"))]
	some_cookie_key: Option<cookie::Key>,
}

impl RequestContextProperties {
	#[cfg(any(feature = "private-cookies", feature = "signed-cookies"))]
	#[inline]
	pub(crate) fn set_cookie_key(&mut self, cookie_key: cookie::Key) {
		self.some_cookie_key = Some(cookie_key);
	}

	#[cfg(any(feature = "private-cookies", feature = "signed-cookies"))]
	#[inline]
	pub(crate) fn clone_cookie_key(&self) -> Option<cookie::Key> {
		self.some_cookie_key.clone()
	}

	#[allow(unused_variables)]
	pub(crate) fn clone_valid_properties_from(&mut self, context_properties: &Self) {
		#[cfg(any(feature = "private-cookies", feature = "signed-cookies"))]
		if context_properties.some_cookie_key.is_some() {
			self
				.some_cookie_key
				.clone_from(&context_properties.some_cookie_key);
		}
	}
}

// --------------------------------------------------
// MistargetedRequest

/// A type that represents a *mistargeted request*.
///
/// ```
/// use argan::{Resource, request::MistargetedRequest};
///
/// let mut resource = Resource::new("/");
/// resource.set_handler_for(MistargetedRequest.to(|| async { /* ... */ }));
/// ```
pub struct MistargetedRequest;

// --------------------------------------------------
// SizeLimit

#[doc(hidden)]
pub enum SizeLimit {
	Default,
	Value(usize),
}

// --------------------------------------------------
// PathParamsError

/// An error type that's returned on failure when extracting path parameters.
///
/// See [`RequestContext::path_params_as()`] and [`RequestHead::path_params_as()`].
#[derive(Debug, crate::ImplError)]
#[error(transparent)]
pub struct PathParamsError(#[from] pub(crate) pattern::DeserializerError);

impl IntoResponse for PathParamsError {
	fn into_response(self) -> Response {
		match self.0 {
			pattern::DeserializerError::ParsingFailue(_) => StatusCode::NOT_FOUND.into_response(),
			_ => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
		}
	}
}

// --------------------------------------------------
// QueryParamsError

/// An error type that's returned on failure when extracting the query string.
///
/// See [`RequestContext::query_params_as()`] and [`RequestHead::query_params_as()`].
#[cfg(feature = "query-params")]
#[derive(Debug, crate::ImplError)]
pub enum QueryParamsError {
	/// Returned when a request doesn't have query params.
	#[error("no data is available")]
	NoDataIsAvailable,
	/// Returned when the deserialization of the query params fails.
	#[error(transparent)]
	InvalidData(#[from] serde_urlencoded::de::Error),
}

#[cfg(feature = "query-params")]
impl IntoResponse for QueryParamsError {
	fn into_response(self) -> Response {
		StatusCode::BAD_REQUEST.into_response()
	}
}

// --------------------------------------------------------------------------------
