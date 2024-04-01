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
use cookie::CookieJar;
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

pub struct RequestContext<B = Body> {
	pub(crate) request: Request<B>,
	pub(crate) routing_state: RoutingState,
}

impl<B> RequestContext<B> {
	#[inline(always)]
	pub(crate) fn new(request: Request<B>, routing_state: RoutingState) -> Self {
		Self {
			request,
			routing_state,
		}
	}

	#[inline(always)]
	pub fn method_ref(&self) -> &Method {
		self.request.method()
	}

	#[inline(always)]
	pub fn method_mut(&mut self) -> &mut Method {
		self.request.method_mut()
	}

	#[inline(always)]
	pub fn uri_ref(&self) -> &Uri {
		self.request.uri()
	}

	#[inline(always)]
	pub fn uri_mut(&mut self) -> &mut Uri {
		self.request.uri_mut()
	}

	#[inline(always)]
	pub fn version(&self) -> Version {
		self.request.version()
	}

	#[inline(always)]
	pub fn version_mut(&mut self) -> &mut Version {
		self.request.version_mut()
	}

	#[inline(always)]
	pub fn headers_ref(&self) -> &HeaderMap<HeaderValue> {
		self.request.headers()
	}

	#[inline(always)]
	pub fn headers_mut(&mut self) -> &mut HeaderMap<HeaderValue> {
		self.request.headers_mut()
	}

	#[inline(always)]
	pub fn extensions_ref(&self) -> &Extensions {
		self.request.extensions()
	}

	#[inline(always)]
	pub fn extensions_mut(&mut self) -> &mut Extensions {
		self.request.extensions_mut()
	}

	#[inline(always)]
	pub fn body_ref(&self) -> &B {
		self.request.body()
	}

	#[inline(always)]
	pub fn body_mut(&mut self) -> &mut B {
		self.request.body_mut()
	}

	#[inline(always)]
	pub fn into_request_parts(self) -> (RequestHead, B) {
		self.request.into_parts()
	}

	// ----------

	pub fn cookies(&self) -> CookieJar {
		todo!()
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
	pub fn remaining_path_segments(&self) -> &str {
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

		request_into_full_body(self.request, size_limit).await
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

		request_into_binary_data(self.request, size_limit).await
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

		request_into_text_data(self.request, size_limit).await
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

		request_into_json_data::<T, B>(self.request, size_limit).await
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

		request_into_form_data::<T, B>(self.request, size_limit).await
	}

	#[inline(always)]
	pub fn into_multipart_form(self) -> Result<MultipartForm, MultipartFormError>
	where
		B: HttpBody<Data = Bytes> + Send + Sync + 'static,
		B::Error: Into<BoxedError>,
	{
		request_into_multipart_form(self.request)
	}

	#[inline(always)]
	pub fn into_websocket_upgrade(self) -> Result<WebSocketUpgrade, WebSocketUpgradeError> {
		request_into_websocket_upgrade(self.request)
	}

	pub async fn extract<'r, T>(&'r self) -> Result<T, T::Error>
	where
		T: FromRequestRef<'r, B>,
	{
		T::from_request_ref(&self.request).await
	}

	pub async fn extract_into<T>(self) -> Result<T, T::Error>
	where
		T: FromRequest<B>,
	{
		T::from_request(self.request).await
	}

	pub fn map<Func, NewB>(self, func: Func) -> RequestContext<NewB>
	where
		Func: FnOnce(B) -> NewB,
	{
		let RequestContext {
			request,
			routing_state,
		} = self;
		let (head, body) = request.into_parts();

		let new_body = func(body);
		let request = Request::from_parts(head, new_body);

		RequestContext {
			request,
			routing_state,
		}
	}
}

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
	pub(crate) fn routing_next_segment_with_index(&mut self) -> Option<(&str, usize)> {
		self
			.routing_state
			.route_traversal
			.next_segment(self.request.uri().path())
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
}

// --------------------------------------------------
// Extract

pub trait Extract<B>: Sized + Sealed {
	fn extract<'r, T>(&'r self) -> impl Future<Output = Result<T, T::Error>> + Send
	where
		T: FromRequestRef<'r, B>;

	fn extract_into<T>(self) -> impl Future<Output = Result<T, T::Error>> + Send
	where
		T: FromRequest<B>;
}

impl<B> Extract<B> for Request<B> {
	fn extract<'r, T>(&'r self) -> impl Future<Output = Result<T, T::Error>>
	where
		T: FromRequestRef<'r, B>,
	{
		T::from_request_ref(self)
	}

	fn extract_into<T>(self) -> impl Future<Output = Result<T, T::Error>>
	where
		T: FromRequest<B>,
	{
		T::from_request(self)
	}
}

impl<B> Sealed for Request<B> {}

// --------------------------------------------------------------------------------
