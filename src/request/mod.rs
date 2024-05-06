//! HTTP request types.

// ----------

use std::{
	borrow::Cow,
	convert::Infallible,
	fmt::{Debug, Display},
	future::{ready, Future, Ready},
};

use argan_core::{
	body::{Body, HttpBody},
	request, BoxedError, BoxedFuture,
};
use bytes::Bytes;
use futures_util::TryFutureExt;
use http::{
	header::{ToStrError, CONTENT_TYPE},
	Extensions, HeaderMap, HeaderName, HeaderValue, Method, StatusCode, Uri, Version,
};
use serde::{
	de::{DeserializeOwned, Error},
	Deserialize, Deserializer,
};

use crate::{
	common::{marker::Sealed, IntoArray, Uncloneable, SCOPE_VALIDITY},
	data::{
		header::{content_type, ContentTypeError},
		request_into_binary_data, request_into_full_body, request_into_text_data, BinaryExtractorError,
		FullBodyExtractorError, TextExtractorError, BINARY_BODY_SIZE_LIMIT, TEXT_BODY_SIZE_LIMIT,
	},
	handler::Args,
	pattern::{self, FromParamsList, ParamsList},
	response::{BoxedErrorResponse, IntoResponse, IntoResponseHeadParts, Response},
	routing::RoutingState,
	ImplError, StdError,
};

#[cfg(feature = "cookies")]
use crate::data::cookies::{cookies_from_request, CookieJar};

#[cfg(feature = "json")]
use crate::data::json::{request_into_json_data, Json, JsonError, JSON_BODY_SIZE_LIMIT};

#[cfg(feature = "form")]
use crate::data::form::{
	request_into_form_data, request_into_multipart_form, FormError, MultipartForm,
	MultipartFormError, FORM_BODY_SIZE_LIMIT,
};

// ----------

pub use argan_core::request::*;

// --------------------------------------------------

#[cfg(feature = "websockets")]
pub mod websocket;

#[cfg(feature = "websockets")]
use self::websocket::{websocket_handshake, WebSocketUpgrade, WebSocketUpgradeError};

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

// --------------------------------------------------
// RequestContext

/// A [`Handler`](crate::handler::Handler) parameter that carries the request data.
pub struct RequestContext<B = Body> {
	request: Request<B>,
	routing_state: RoutingState,
	properties: ContextProperties,
}

impl<B> RequestContext<B> {
	#[inline(always)]
	pub(crate) fn new(
		request: Request<B>,
		routing_state: RoutingState,
		properties: ContextProperties,
	) -> Self {
		Self {
			request,
			routing_state,
			properties,
		}
	}

	#[inline(always)]
	pub(crate) fn clone_valid_properties_from(&mut self, mut context_properties: &ContextProperties) {
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

	/// Returns the request cookies.
	#[cfg(feature = "cookies")]
	pub fn cookies(&mut self) -> CookieJar {
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

	/// Returns the remaining segments of the request's path.
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

	/// Consumes the `RequestContext`, collects the request body and returns it as [`Bytes`].
	#[doc(hidden)]
	pub async fn into_full_body(self, size_limit: SizeLimit) -> Result<Bytes, FullBodyExtractorError>
	where
		B: HttpBody,
		B::Error: Into<BoxedError>,
	{
		let size_limit = match size_limit {
			SizeLimit::Default => BINARY_BODY_SIZE_LIMIT,
			SizeLimit::Value(value) => value,
		};

		let (_, body) = self.request.into_parts();

		request_into_full_body(body, size_limit).await
	}

	/// Consumes the `RequestContext`, collects the request body if the `Content-Type` is
	/// either `octet-stream` or `application/octet-stream` and returns it as [`Bytes`].
	#[doc(hidden)]
	pub async fn into_binary_data(self, size_limit: SizeLimit) -> Result<Bytes, BinaryExtractorError>
	where
		B: HttpBody,
		B::Error: Into<BoxedError>,
	{
		let size_limit = match size_limit {
			SizeLimit::Default => BINARY_BODY_SIZE_LIMIT,
			SizeLimit::Value(value) => value,
		};

		let (head_parts, body) = self.request.into_parts();

		request_into_binary_data(&head_parts, body, size_limit).await
	}

	/// Consumes the `RequestContext`, collects the request body if the `Content-Type` is
	/// either `text/plain` or `text/plain; charset=utf-8` and returns it converted to
	/// [`String`].
	#[doc(hidden)]
	pub async fn into_text_data(self, size_limit: SizeLimit) -> Result<String, TextExtractorError>
	where
		B: HttpBody,
		B::Error: Into<BoxedError>,
	{
		let size_limit = match size_limit {
			SizeLimit::Default => TEXT_BODY_SIZE_LIMIT,
			SizeLimit::Value(value) => value,
		};

		let (head_parts, body) = self.request.into_parts();

		request_into_text_data(&head_parts, body, size_limit).await
	}

	/// Consumes the `RequestContext`, collects the request body if the `Content-Type` is
	/// `application/json` and returns it deserialized as type `T`. `T` must implement
	/// [`serde::Deserialize`].
	#[cfg(feature = "json")]
	#[doc(hidden)]
	pub async fn into_json_data<T>(self, size_limit: SizeLimit) -> Result<T, JsonError>
	where
		B: HttpBody,
		B::Error: Into<BoxedError>,
		T: DeserializeOwned,
	{
		let size_limit = match size_limit {
			SizeLimit::Default => JSON_BODY_SIZE_LIMIT,
			SizeLimit::Value(value) => value,
		};

		let (head_parts, body) = self.request.into_parts();

		request_into_json_data::<T, B>(&head_parts, body, size_limit).await
	}

	/// Consumes the `RequestContext`, collects the request body if the `Content-Type` is
	/// `application/x-www-form-urlencoded` and returns it deserialized as type `T`. `T`
	/// must implement [`serde::Deserialize`].
	#[cfg(feature = "form")]
	#[doc(hidden)]
	pub async fn into_form_data<T>(self, size_limit: SizeLimit) -> Result<T, FormError>
	where
		B: HttpBody,
		B::Error: Into<BoxedError>,
		T: DeserializeOwned,
	{
		let size_limit = match size_limit {
			SizeLimit::Default => FORM_BODY_SIZE_LIMIT,
			SizeLimit::Value(value) => value,
		};

		let (head_parts, body) = self.request.into_parts();

		request_into_form_data::<T, B>(&head_parts, body, size_limit).await
	}

	/// Consumes the `RequestContext` and returns a `multipart/form-data` extractor.
	#[doc(hidden)]
	#[cfg(feature = "multpart-form")]
	#[inline(always)]
	pub fn into_multipart_form(self) -> Result<MultipartForm<B>, MultipartFormError>
	where
		B: HttpBody<Data = Bytes> + Send + Sync + 'static,
		B::Error: Into<BoxedError>,
	{
		let (head_parts, body) = self.request.into_parts();

		request_into_multipart_form(&head_parts, body)
	}

	/// Consumes the `RequestContext` and returns an extractor to establish a WebSocket connection.
	#[cfg(feature = "websockets")]
	#[doc(hidden)]
	#[inline(always)]
	pub fn into_websocket_upgrade(self) -> Result<WebSocketUpgrade, WebSocketUpgradeError> {
		let (mut head, _) = self.request.into_parts();

		websocket_handshake(&mut head)
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
		let mut request_head = RequestHead::new(head_parts, self.routing_state, self.properties);

		(request_head, result)
	}

	/// Calls the given function to map the request body and returns the `RequestContext`
	/// with the mapped body.
	pub fn map<Func, NewB>(self, func: Func) -> RequestContext<NewB>
	where
		Func: FnOnce(B) -> NewB,
	{
		let RequestContext {
			request,
			routing_state,
			properties,
		} = self;

		let (head, body) = request.into_parts();

		let new_body = func(body);
		let request = Request::from_parts(head, new_body);

		RequestContext {
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
	pub(crate) fn routing_host_and_uri_params_mut(&mut self) -> Option<(&str, &mut ParamsList)> {
		let host = self.request.uri().host()?;

		Some((host, &mut self.routing_state.uri_params))
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

	#[inline(always)]
	pub(crate) fn into_parts(self) -> (Request<B>, RoutingState, ContextProperties) {
		(self.request, self.routing_state, self.properties)
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

// --------------------------------------------------
// RequestHeead

/// A type of request's head part.
pub struct RequestHead {
	method: Method,
	uri: Uri,
	version: Version,
	headers: HeaderMap<HeaderValue>,
	extensions: Extensions,

	routing_state: RoutingState,
	context_properties: ContextProperties,
}

impl RequestHead {
	#[inline(always)]
	pub(crate) fn new(
		head_parts: RequestHeadParts,
		routing_state: RoutingState,
		context_properties: ContextProperties,
	) -> Self {
		Self {
			method: head_parts.method,
			uri: head_parts.uri,
			version: head_parts.version,
			headers: head_parts.headers,
			extensions: head_parts.extensions,
			routing_state,
			context_properties,
		}
	}

	// #[cfg(any(feature = "private-cookies", feature = "signed-cookies"))]
	// #[inline(always)]
	// pub(crate) fn clone_cookie_key(&self) -> Option<cookie::Key> {
	// 	self.context_properties.some_cookie_key.clone()
	// }
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

	/// Returns the request cookies.
	#[cfg(feature = "cookies")]
	#[inline(always)]
	pub fn cookies(&mut self) -> CookieJar {
		cookies_from_request(
			&self.headers,
			#[cfg(any(feature = "private-cookies", feature = "signed-cookies"))]
			self.context_properties.clone_cookie_key(),
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

	/// Returns the remaining segments of the request's path.
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
pub(crate) struct ContextProperties {
	#[cfg(any(feature = "private-cookies", feature = "signed-cookies"))]
	some_cookie_key: Option<cookie::Key>,
}

impl ContextProperties {
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

	pub(crate) fn clone_valid_properties_from(&mut self, mut context_properties: &Self) {
		#[cfg(any(feature = "private-cookies", feature = "signed-cookies"))]
		if context_properties.some_cookie_key.is_some() {
			self
				.some_cookie_key
				.clone_from(&context_properties.some_cookie_key);
		}
	}
}

// --------------------------------------------------
// SizeLimit

#[doc(hidden)]
pub enum SizeLimit {
	Default,
	Value(usize),
}

// --------------------------------------------------
// PathParamsError

/// An error type returned by [`RequestContext::path_params_as()`] and
/// [`RequestHead::path_params_as()`].
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

/// An error type returned by [`RequestContext::query_params_as()`] and
/// [`RequestHead::query_params_as()`].
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

// --------------------------------------------------

impl IntoArray<Method, 1> for Method {
	fn into_array(self) -> [Method; 1] {
		[self]
	}
}

// --------------------------------------------------------------------------------
