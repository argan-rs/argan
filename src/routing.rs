use crate::{request::Request, resource::ResourceService, utils::RouteTraversalState};

pub use hyper::Method;
pub use hyper::StatusCode;

// --------------------------------------------------

pub(crate) struct RoutingState {
	pub(crate) path_traversal: RouteTraversalState,
	pub(crate) current_resource: Option<ResourceService>,

	pub(crate) subtree_handler_exists: bool,
}

impl RoutingState {
	pub(crate) fn new(
		path_segments: RouteTraversalState,
		resource_service: ResourceService,
	) -> RoutingState {
		Self {
			path_traversal: path_segments,
			current_resource: Some(resource_service),
			subtree_handler_exists: false,
		}
	}
}

// --------------------------------------------------

pub(crate) struct UnusedRequest(Request);

impl From<Request> for UnusedRequest {
	fn from(value: Request) -> Self {
		UnusedRequest(value)
	}
}

impl From<UnusedRequest> for Request {
	fn from(value: UnusedRequest) -> Self {
		value.0
	}
}

impl AsRef<Request> for UnusedRequest {
	fn as_ref(&self) -> &Request {
		&self.0
	}
}

impl AsMut<Request> for UnusedRequest {
	fn as_mut(&mut self) -> &mut Request {
		&mut self.0
	}
}

impl UnusedRequest {
	#[inline]
	pub(crate) fn into_request(self) -> Request {
		self.0
	}
}
