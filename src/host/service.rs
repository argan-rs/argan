use std::{convert::Infallible, future::ready};

use http::StatusCode;
use hyper::service::Service;

use crate::{
	body::{Bytes, HttpBody},
	common::{BoxedError, BoxedFuture},
	handler::{futures::ResponseToResultFuture, request_handlers::handle_mistargeted_request},
	pattern::ParamsList,
	request::Request,
	resource::ResourceService,
	response::{IntoResponse, Response},
};

use super::*;

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

pub struct HostService {
	pattern: Pattern,
	root_resource: ResourceService,
}

impl HostService {
	pub(super) fn new(pattern: Pattern, root_resource: ResourceService) -> Self {
		Self {
			pattern,
			root_resource,
		}
	}

	#[inline(always)]
	pub(crate) fn is_static_match(&self, text: &str) -> Option<bool> {
		self.pattern.is_static_match(text)
	}

	#[inline(always)]
	pub(crate) fn is_regex_match(&self, text: &str, params_list: &mut ParamsList) -> Option<bool> {
		self.pattern.is_regex_match(text, params_list)
	}

	#[inline(always)]
	pub(crate) fn handle_with_params<B>(
		&self,
		request: Request<B>,
		uri_params: ParamsList,
	) -> BoxedFuture<Response>
	where
		B: HttpBody<Data = Bytes> + Send + Sync + 'static,
		B::Error: Into<BoxedError>,
	{
		self.root_resource.handle_with_params(request, uri_params)
	}
}

impl<B> Service<Request<B>> for HostService
where
	B: HttpBody<Data = Bytes> + Send + Sync + 'static,
	B::Error: Into<BoxedError>,
{
	type Response = Response;
	type Error = Infallible;
	type Future = ResponseToResultFuture<BoxedFuture<Response>>;

	fn call(&self, request: Request<B>) -> Self::Future {
		macro_rules! handle_unmatching_host {
			() => {
				ResponseToResultFuture::from(
					Box::pin(ready(StatusCode::NOT_FOUND.into_response())) as BoxedFuture<Response>
				)
			};
		}

		let Some(host) = request.uri().host() else {
			return handle_unmatching_host!();
		};

		let mut uri_params = ParamsList::new();

		if let Some(result) = self.pattern.is_static_match(host) {
			return ResponseToResultFuture::from(
				self.root_resource.handle_with_params(request, uri_params),
			);
		} else {
			if let Some(result) = self.pattern.is_regex_match(host, &mut uri_params) {
				return ResponseToResultFuture::from(
					self.root_resource.handle_with_params(request, uri_params),
				);
			}
		}

		handle_unmatching_host!()
	}
}

// --------------------------------------------------------------------------------
