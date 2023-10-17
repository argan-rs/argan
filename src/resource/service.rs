use std::{any::Any, convert::Infallible, fmt::Debug, sync::Arc};

use percent_encoding::percent_decode_str;

use crate::{
	body::{Body, IncomingBody},
	handler::{
		request_handlers::{misdirected_request_handler, MethodHandlers},
		ArcHandler, Handler, Service,
	},
	pattern::{ParamsList, Pattern},
	request::Request,
	response::Response,
	routing::{RouteTraversal, RoutingState},
	utils::{BoxedError, BoxedFuture},
};

use super::futures::{
	RequestPasserFuture, RequestReceiverFuture, ResourceFuture, ResourceInternalFuture,
};

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

#[derive(Clone)]
pub struct ResourceService {
	pub(super) pattern: Pattern,

	pub(super) static_resources: Option<Arc<[ResourceService]>>,
	pub(super) regex_resources: Option<Arc<[ResourceService]>>,
	pub(super) wildcard_resource: Option<Arc<ResourceService>>,

	pub(super) request_receiver: Option<ArcHandler>,
	pub(super) request_passer: Option<ArcHandler>,
	pub(super) request_handler: Option<ArcHandler>,

	pub(super) method_handlers: MethodHandlers,

	pub(super) state: Option<Arc<[Box<dyn Any + Send + Sync>]>>,

	// TODO: configs, state, redirect, parent
	pub(super) is_subtree_handler: bool,
}

impl ResourceService {
	#[inline]
	pub(super) fn is_subtree_handler(&self) -> bool {
		self.is_subtree_handler
	}

	#[inline]
	pub(super) fn can_handle_request(&self) -> bool {
		self.method_handlers.is_empty()
	}
}

// --------------------------------------------------

impl<B> Service<Request<B>> for ResourceService
where
	B: Body + Send + Sync + 'static,
	B::Data: Debug,
	B::Error: Into<BoxedError>,
{
	type Response = Response;
	type Error = Infallible;
	type Future = ResourceFuture;

	#[inline]
	fn call(&self, request: Request<B>) -> Self::Future {
		let (head, body) = request.into_parts();
		let incoming_body = IncomingBody::new(body);
		let mut request = Request::<IncomingBody>::from_parts(head, incoming_body);

		let route = request.uri().path();
		let mut route_traversal = RouteTraversal::for_route(route);
		let mut path_params = ParamsList::new();

		let matched = if route == "/" {
			if let Some(result) = self.pattern.is_static_match(route) {
				result
			} else {
				false
			}
		} else {
			let (next_segment, _) = route_traversal.next_segment(request.uri().path()).unwrap();

			if let Some(result) = self.pattern.is_static_match(next_segment) {
				result
			} else {
				let decoded_segment =
					Arc::<str>::from(percent_decode_str(next_segment).decode_utf8().unwrap());

				if let Some(result) = self
					.pattern
					.is_regex_match(decoded_segment.clone(), &mut path_params)
				{
					result
				} else {
					self
						.pattern
						.is_wildcard_match(decoded_segment, &mut path_params)
						.unwrap()
				}
			}
		};

		let routing_state = RoutingState::new(route_traversal, self.clone());
		request.extensions_mut().insert(routing_state);

		if
		/*matched*/
		true {
			match self.request_receiver.as_ref() {
				Some(request_receiver) => {
					ResourceInternalFuture::from(request_receiver.handle(request)).into()
				}
				None => ResourceInternalFuture::from(request_receiver(request)).into(),
			}
		} else {
			ResourceInternalFuture::from(misdirected_request_handler(request)).into()
		}
	}
}

// --------------------------------------------------------------------------------

#[inline]
pub(super) fn request_receiver(mut request: Request) -> RequestReceiverFuture {
	RequestReceiverFuture::from(request)
}

#[inline]
pub(super) fn request_passer(mut request: Request) -> RequestPasserFuture {
	RequestPasserFuture::from(request)
}

pub(super) fn request_handler(mut request: Request) -> BoxedFuture<Response> {
	let routing_state = request.extensions_mut().get_mut::<RoutingState>().unwrap();
	let current_resource = routing_state.current_resource.take().unwrap();

	current_resource.method_handlers.handle(request)
}

#[cfg(test)]
mod test {
	use std::str::FromStr;

	use http::{Method, StatusCode, Uri};

	use crate::{
		body::{Bytes, Empty},
		resource::Resource,
	};

	use super::*;

	// --------------------------------------------------

	#[tokio::test]
	async fn resource_service() {
		let mut resource = Resource::new("/abc0_0");
		resource.set_handler(Method::GET, hello_world);

		resource.set_subresource_handler("/*abc1_0", Method::PUT, hello_world);
		resource.set_subresource_handler("/*abc1_0/$abc2_0:@(abc2_0)", Method::POST, hello_world);
		resource.set_subresource_handler(
			"/*abc1_0/$abc2_1:@cn(abc2_1)-cba/*abc3_0",
			Method::GET,
			hello_world,
		);

		resource.set_subresource_handler("/$abc1_1:@cn(abc1_1)-cba", Method::GET, hello_world);
		resource.set_subresource_handler("/$abc1_1:@cn(abc1_1)-cba/abc2_0", Method::GET, hello_world);

		dbg!();

		let service = resource.into_service();

		dbg!();

		let request = new_request("GET", "/abc0_0");
		let response = service.call(request).await.unwrap();
		assert_eq!(response.status(), StatusCode::OK);

		dbg!();

		let request = new_request("PUT", "/abc0_0/abc1_0");
		let response = service.call(request).await.unwrap();
		assert_eq!(response.status(), StatusCode::OK);

		dbg!();

		let request = new_request("POST", "/abc0_0/*abc1_0/abc2_0");
		let response = service.call(request).await.unwrap();
		assert_eq!(response.status(), StatusCode::OK);

		dbg!();

		let request = new_request("GET", "/abc0_0/abc1_1-cba");
		let response = service.call(request).await.unwrap();
		assert_eq!(response.status(), StatusCode::OK);

		dbg!();

		let request = new_request("GET", "/abc0_0/abc1_1-cba/abc2_0");
		let response = service.call(request).await.unwrap();
		assert_eq!(response.status(), StatusCode::OK);

		dbg!();

		let request = new_request("GET", "/abc0_0/abc1_0-wildcard/abc2_1-cba/wildcard-abc3_0");
		let response = service.call(request).await.unwrap();
		assert_eq!(response.status(), StatusCode::OK);
	}

	fn new_request(method: &str, uri: &str) -> Request<Empty<Bytes>> {
		let mut request = Request::new(Empty::<Bytes>::new());
		*request.method_mut() = Method::from_str(method).unwrap();
		*request.uri_mut() = Uri::from_str(uri).unwrap();

		request
	}

	async fn hello_world() -> &'static str {
		"Hello, World!"
	}
}
