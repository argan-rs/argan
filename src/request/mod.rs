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
use cookie::Key;
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
	common::{marker::Sealed, Uncloneable},
	data::{
		cookie::{cookies_from_request, CookieJar},
		form::{
			request_into_form_data, request_into_multipart_form, FormError, MultipartForm,
			MultipartFormError, FORM_BODY_SIZE_LIMIT,
		},
		header::{content_type, ContentTypeError},
		json::{request_into_json_data, Json, JsonError, JSON_BODY_SIZE_LIMIT},
		request_into_binary_data, request_into_full_body, request_into_text_data, BinaryExtractorError,
		FullBodyExtractorError, TextExtractorError, BODY_SIZE_LIMIT,
	},
	handler::Args,
	pattern::{self, FromParamsList, ParamsList},
	response::{BoxedErrorResponse, IntoResponse, IntoResponseHead, Response},
	routing::RoutingState,
	ImplError, StdError,
};

// ----------

pub use argan_core::request::*;

// --------------------------------------------------

pub mod websocket;

mod extractors;
pub use extractors::*;

use self::websocket::{request_into_websocket_upgrade, WebSocketUpgrade, WebSocketUpgradeError};

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

// --------------------------------------------------
// RequestContext

pub struct RequestContext<B = Body> {
	request: Request<B>,
	routing_state: RoutingState,
	some_cookie_key: Option<cookie::Key>,
}

impl<B> RequestContext<B> {
	#[inline(always)]
	pub(crate) fn new(request: Request<B>, routing_state: RoutingState) -> Self {
		Self {
			request,
			routing_state,
			some_cookie_key: None,
		}
	}

	#[inline(always)]
	pub(crate) fn with_cookie_key(mut self, cookie_key: cookie::Key) -> Self {
		self.some_cookie_key = Some(cookie_key);

		self
	}

	#[inline(always)]
	pub fn method_ref(&self) -> &Method {
		self.request.method()
	}

	#[inline(always)]
	pub fn uri_ref(&self) -> &Uri {
		self.request.uri()
	}

	#[inline(always)]
	pub fn version(&self) -> Version {
		self.request.version()
	}

	#[inline(always)]
	pub fn headers_ref(&self) -> &HeaderMap<HeaderValue> {
		self.request.headers()
	}

	#[inline(always)]
	pub fn extensions_ref(&self) -> &Extensions {
		self.request.extensions()
	}

	// ----------

	#[inline(always)]
	pub fn request_mut(&mut self) -> &mut Request<B> {
		&mut self.request
	}

	// ----------

	// Consumes cookies.
	pub fn cookies(&mut self) -> CookieJar {
		cookies_from_request(self.headers_ref(), self.some_cookie_key.clone())
	}

	#[inline]
	pub fn path_params_as<'r, T>(&'r self) -> Result<T, PathParamsError>
	where
		T: Deserialize<'r>,
	{
		let mut from_params_list = self.routing_state.uri_params.deserializer();

		T::deserialize(&mut from_params_list)
			.map(|value| value)
			.map_err(Into::into)
	}

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

		serde_urlencoded::from_str::<T>(query_string)
			.map(|value| value)
			.map_err(|error| QueryParamsError::InvalidData(error.into()))
	}

	#[inline(always)]
	pub fn subtree_path_segments(&self) -> &str {
		self
			.routing_state
			.route_traversal
			.remaining_segments(self.request.uri().path())
	}

	pub async fn into_full_body(self, size_limit: SizeLimit) -> Result<Bytes, FullBodyExtractorError>
	where
		B: HttpBody,
		B::Error: Into<BoxedError>,
	{
		let size_limit = match size_limit {
			SizeLimit::Default => BODY_SIZE_LIMIT,
			SizeLimit::Value(value) => value,
		};

		let (_, body) = self.request.into_parts();

		request_into_full_body(body, size_limit).await
	}

	pub async fn into_binary_data(self, size_limit: SizeLimit) -> Result<Bytes, BinaryExtractorError>
	where
		B: HttpBody,
		B::Error: Into<BoxedError>,
	{
		let size_limit = match size_limit {
			SizeLimit::Default => BODY_SIZE_LIMIT,
			SizeLimit::Value(value) => value,
		};

		let (head_parts, body) = self.request.into_parts();

		request_into_binary_data(&head_parts, body, size_limit).await
	}

	pub async fn into_text_data(self, size_limit: SizeLimit) -> Result<String, TextExtractorError>
	where
		B: HttpBody,
		B::Error: Into<BoxedError>,
	{
		let size_limit = match size_limit {
			SizeLimit::Default => BODY_SIZE_LIMIT,
			SizeLimit::Value(value) => value,
		};

		let (head_parts, body) = self.request.into_parts();

		request_into_text_data(&head_parts, body, size_limit).await
	}

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

	#[inline(always)]
	pub fn into_multipart_form(self) -> Result<MultipartForm<B>, MultipartFormError>
	where
		B: HttpBody<Data = Bytes> + Send + Sync + 'static,
		B::Error: Into<BoxedError>,
	{
		let (head_parts, body) = self.request.into_parts();

		request_into_multipart_form(&head_parts, body)
	}

	#[inline(always)]
	pub fn into_websocket_upgrade(self) -> Result<WebSocketUpgrade, WebSocketUpgradeError> {
		let (mut head, _) = self.request.into_parts();

		request_into_websocket_upgrade(&mut head)
	}

	pub async fn extract<'r, T>(&'r self) -> Result<T, T::Error>
	where
		T: FromRequestRef<'r, B>,
	{
		T::from_request_ref(&self.request).await
	}

	pub async fn extract_into<T>(self) -> (RequestHead, Result<T, T::Error>)
	where
		T: FromRequest<B>,
	{
		let (head_parts, body) = self.request.into_parts();
		let (head_parts, result) = T::from_request(head_parts, body).await;

		let request_head = RequestHead::new(head_parts, self.routing_state);

		(request_head, result)
	}

	pub fn map<Func, NewB>(self, func: Func) -> RequestContext<NewB>
	where
		Func: FnOnce(B) -> NewB,
	{
		let RequestContext {
			request,
			routing_state,
			some_cookie_key,
		} = self;
		let (head, body) = request.into_parts();

		let new_body = func(body);
		let request = Request::from_parts(head, new_body);

		RequestContext {
			request,
			routing_state,
			some_cookie_key,
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
		let Some(host) = self.request.uri().host() else {
			return None;
		};

		Some((host, &mut self.routing_state.uri_params))
	}

	#[inline(always)]
	pub(crate) fn routing_next_segment_and_uri_params_mut(
		&mut self,
	) -> Option<(&str, &mut ParamsList)> {
		let Some((next_segment, _)) = self
			.routing_state
			.route_traversal
			.next_segment(self.request.uri().path())
		else {
			return None;
		};

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
	pub(crate) fn into_parts(self) -> (Request<B>, RoutingState, Option<cookie::Key>) {
		(self.request, self.routing_state, self.some_cookie_key)
	}
}

// --------------------------------------------------
// RequestHeead

pub struct RequestHead {
	method: Method,
	uri: Uri,
	version: Version,
	headers: HeaderMap<HeaderValue>,
	extensions: Extensions,

	routing_state: RoutingState,
	some_cookie_key: Option<cookie::Key>,
}

impl RequestHead {
	#[inline(always)]
	pub(crate) fn new(head_parts: RequestHeadParts, routing_state: RoutingState) -> Self {
		Self {
			method: head_parts.method,
			uri: head_parts.uri,
			version: head_parts.version,
			headers: head_parts.headers,
			extensions: head_parts.extensions,
			routing_state,
			some_cookie_key: None,
		}
	}

	#[inline(always)]
	pub(crate) fn with_cookie_key(mut self, cookie_key: cookie::Key) -> Self {
		self.some_cookie_key = Some(cookie_key);

		self
	}

	#[inline(always)]
	pub(crate) fn take_some_cookie_key(&mut self) -> Option<cookie::Key> {
		self.some_cookie_key.take()
	}
}

impl RequestHead {
	#[inline(always)]
	pub fn method_ref(&self) -> &Method {
		&self.method
	}

	#[inline(always)]
	pub fn set_method(&mut self, method: Method) {
		self.method = method;
	}

	#[inline(always)]
	pub fn uri_ref(&self) -> &Uri {
		&self.uri
	}

	#[inline(always)]
	pub fn set_uri(&mut self, uri: Uri) {
		self.uri = uri;
	}

	#[inline(always)]
	pub fn version(&self) -> Version {
		self.version
	}

	#[inline(always)]
	pub fn set_version(&mut self, version: Version) {
		self.version = version;
	}

	#[inline(always)]
	pub fn headers_ref(&self) -> &HeaderMap<HeaderValue> {
		&self.headers
	}

	#[inline(always)]
	pub fn headers_mut(&mut self) -> &mut HeaderMap<HeaderValue> {
		&mut self.headers
	}

	#[inline(always)]
	pub fn extensions_ref(&self) -> &Extensions {
		&self.extensions
	}

	#[inline(always)]
	pub fn extensions_mut(&mut self) -> &mut Extensions {
		&mut self.extensions
	}

	// ----------

	#[inline(always)]
	pub fn cookies(&mut self) -> CookieJar {
		cookies_from_request(&self.headers, self.some_cookie_key.clone())
	}

	#[inline]
	pub fn path_params_as<'r, T>(&'r self) -> Result<T, PathParamsError>
	where
		T: Deserialize<'r>,
	{
		let mut from_params_list = self.routing_state.uri_params.deserializer();

		T::deserialize(&mut from_params_list)
			.map(|value| value)
			.map_err(Into::into)
	}

	#[inline]
	pub fn query_params_as<'r, T>(&'r self) -> Result<T, QueryParamsError>
	where
		T: Deserialize<'r>,
	{
		let query_string = self
			.uri
			.query()
			.ok_or(QueryParamsError::NoDataIsAvailable)?;

		serde_urlencoded::from_str::<T>(query_string)
			.map(|value| value)
			.map_err(|error| QueryParamsError::InvalidData(error.into()))
	}

	#[inline(always)]
	pub fn subtree_path_segments(&self) -> &str {
		self
			.routing_state
			.route_traversal
			.remaining_segments(self.uri.path())
	}
}

// --------------------------------------------------
// SizeLimit

pub enum SizeLimit {
	Default,
	Value(usize),
}

// --------------------------------------------------------------------------------
