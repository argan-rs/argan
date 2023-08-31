use std::{any::Any, convert::Infallible, sync::Arc};

use crate::{
	body::{Incoming, IncomingBody},
	handler::{
		request_handlers::{misdirected_request_handler, MethodHandlers},
		ArcHandler, Handler, Service,
	},
	pattern::Pattern,
	request::Request,
	response::Response,
	routing::{RouteTraversal, RoutingState},
	utils::BoxedFuture,
};

use super::futures::{
	RequestPasserFuture, RequestReceiverFuture, ResourceFuture, ResourceInternalFuture,
};

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

#[derive(Clone)]
pub struct ResourceService {
	pub(super) pattern: Pattern,

	pub(super) static_resources: Arc<[ResourceService]>,
	pub(super) regex_resources: Arc<[ResourceService]>,
	pub(super) wildcard_resource: Option<Arc<ResourceService>>,

	pub(super) request_receiver: Option<ArcHandler>,
	pub(super) request_passer: Option<ArcHandler>,
	pub(super) request_handler: Option<ArcHandler>,

	pub(super) method_handlers: MethodHandlers,

	pub(super) state: Arc<[Box<dyn Any + Send + Sync>]>,

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

impl Service<Request<Incoming>> for ResourceService {
	type Response = Response;
	type Error = Infallible;
	type Future = ResourceFuture;

	#[inline]
	fn call(&self, request: Request<Incoming>) -> Self::Future {
		let (head, body) = request.into_parts();
		let incoming_body = IncomingBody::new(body);
		let mut request = Request::<IncomingBody>::from_parts(head, incoming_body);

		let route = request.uri().path();
		let mut route_traversal = RouteTraversal::new();

		let matched = if route == "/" {
			self.pattern.is_match(route)
		} else {
			let (next_segment, _) = route_traversal.next_segment(request.uri().path()).unwrap();

			self.pattern.is_match(next_segment)
		};

		let routing_state = RoutingState::new(route_traversal, self.clone());
		request.extensions_mut().insert(routing_state);

		if matched {
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
	let current_resource = routing_state.current_resource.take().unwrap(); // ???

	current_resource.method_handlers.handle(request)
}
