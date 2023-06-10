use crate::{
	resource::Resource,
	utils::{PathSegments, Request},
};

// --------------------------------------------------

pub(crate) struct RoutingState<'req> {
	pub(crate) path_segments: PathSegments<'req>,
	pub(crate) current_resource: Option<&'req Resource>,

	pub(crate) subtree_handler_exists: bool,
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