use std::{convert::Infallible, future::ready};

use http::{Extensions, StatusCode};
use hyper::service::Service;

use crate::{
	body::{Bytes, HttpBody},
	common::{BoxedError, BoxedFuture},
	handler::{futures::ResponseToResultFuture, request_handlers::handle_mistargeted_request, Args},
	pattern::ParamsList,
	request::Request,
	resource::{ResourceExtensions, ResourceService},
	response::{IntoResponse, Response},
	routing::{RouteTraversal, RoutingState},
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
	pub(crate) fn handle<B>(&self, request: Request<B>, args: &mut Args) -> BoxedFuture<Response>
	where
		B: HttpBody<Data = Bytes> + Send + Sync + 'static,
		B::Error: Into<BoxedError>,
	{
		self.root_resource.handle(request, args)
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

		let routing_state = RoutingState::new(RouteTraversal::for_route(request.uri().path()));
		let mut args = Args {
			routing_state,
			resource_extensions: ResourceExtensions::new_owned(Extensions::new()),
			handler_extension: &(),
		};

		if let Some(result) = self.pattern.is_static_match(host) {
			return ResponseToResultFuture::from(self.root_resource.handle(request, &mut args));
		} else {
			if let Some(result) = self
				.pattern
				.is_regex_match(host, &mut args.routing_state.uri_params)
			{
				return ResponseToResultFuture::from(self.root_resource.handle(request, &mut args));
			}
		}

		handle_unmatching_host!()
	}
}

// --------------------------------------------------------------------------------
