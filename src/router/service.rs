use std::{convert::Infallible, future::ready, sync::Arc};

use http::{Extensions, StatusCode};
use hyper::service::Service;

use crate::{
	body::{Body, Bytes, HttpBody},
	common::{BoxedError, BoxedFuture, MaybeBoxed},
	data::extensions::NodeExtensions,
	handler::{futures::ResponseToResultFuture, Args, BoxedHandler, Handler},
	host::{Host, HostService},
	middleware::Layer,
	pattern::ParamsList,
	request::Request,
	resource::{Resource, ResourceService},
	response::{BoxedErrorResponse, IntoResponse, Response},
	routing::{RouteTraversal, RoutingState},
};

use super::RouterLayerTarget;

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

pub struct RouterService {
	extensions: Extensions,
	request_passer: MaybeBoxed<RequestPasser>,
}

impl RouterService {
	#[inline(always)]
	pub(super) fn new(extensions: Extensions, request_passer: MaybeBoxed<RequestPasser>) -> Self {
		Self {
			extensions,
			request_passer,
		}
	}
}

impl<B> Service<Request<B>> for RouterService
where
	B: HttpBody<Data = Bytes> + Send + Sync + 'static,
	B::Error: Into<BoxedError>,
{
	type Response = Response;
	type Error = BoxedErrorResponse;
	type Future = BoxedFuture<Result<Self::Response, Self::Error>>;

	fn call(&self, mut request: Request<B>) -> Self::Future {
		let routing_state = RoutingState::new(RouteTraversal::for_route(request.uri().path()));
		let mut args = Args {
			routing_state,
			node_extensions: NodeExtensions::new_borrowed(&self.extensions),
			handler_extension: &(),
		};

		match &self.request_passer {
			MaybeBoxed::Boxed(boxed_request_passer) => {
				boxed_request_passer.handle(request.map(Body::new), &mut args)
			}
			MaybeBoxed::Unboxed(request_passer) => request_passer.handle(request, &mut args),
		}
	}
}

// --------------------------------------------------
// RequestPasser

#[derive(Clone)]
pub(super) struct RequestPasser {
	some_static_hosts: Option<Arc<[HostService]>>,
	some_regex_hosts: Option<Arc<[HostService]>>,
	some_root_resource: Option<Arc<ResourceService>>,
}

impl RequestPasser {
	pub(super) fn new(
		some_static_hosts: Option<Arc<[HostService]>>,
		some_regex_hosts: Option<Arc<[HostService]>>,
		some_root_resource: Option<Arc<ResourceService>>,
		middleware: Vec<RouterLayerTarget>,
	) -> MaybeBoxed<Self> {
		let request_passer = Self {
			some_static_hosts,
			some_regex_hosts,
			some_root_resource,
		};

		let mut maybe_boxed_request_passer = MaybeBoxed::Unboxed(request_passer);

		for mut layer in middleware.into_iter().rev() {
			use super::RouterLayerTargetValue;

			match layer.0 {
				RouterLayerTargetValue::RequestPasser(_) => {
					let RouterLayerTargetValue::RequestPasser(boxed_layer) = layer.0.take() else {
						unreachable!()
					};

					match maybe_boxed_request_passer {
						MaybeBoxed::Boxed(boxed_request_passer) => {
							maybe_boxed_request_passer =
								MaybeBoxed::Boxed(boxed_layer.wrap(boxed_request_passer.into()));
						}
						MaybeBoxed::Unboxed(request_passer) => {
							let mut boxed_requst_passer = BoxedHandler::new(request_passer);

							maybe_boxed_request_passer =
								MaybeBoxed::Boxed(boxed_layer.wrap(boxed_requst_passer.into()));
						}
					}
				}
				_ => {}
			}
		}

		maybe_boxed_request_passer
	}
}

impl<B> Handler<B> for RequestPasser
where
	B: HttpBody<Data = Bytes> + Send + Sync + 'static,
	B::Error: Into<BoxedError>,
{
	type Response = Response;
	type Error = BoxedErrorResponse;
	type Future = BoxedFuture<Result<Self::Response, Self::Error>>;

	fn handle(&self, request: Request<B>, args: &mut Args) -> Self::Future {
		if let Some(uri_host) = request.uri().host() {
			if let Some(host) = self.some_static_hosts.as_ref().and_then(|hosts| {
				hosts.iter().find(|host| {
					host
						.is_static_match(uri_host)
						.expect("static_hosts must keep only the hosts with a static pattern")
				})
			}) {
				return host.handle(request, args);
			}

			if let Some(host) = self.some_regex_hosts.as_ref().and_then(|hosts| {
				hosts.iter().find(|host| {
					host
						.is_regex_match(uri_host, &mut args.routing_state.uri_params)
						.expect("regex_hosts must keep only the hosts with a static pattern")
				})
			}) {
				return host.handle(request, args);
			}
		}

		if let Some(root_resource) = self.some_root_resource.as_deref() {
			return root_resource.handle(request, args);
		}

		Box::pin(ready(Ok(StatusCode::NOT_FOUND.into_response())))
	}
}
