use hyper::StatusCode;

use super::utils::*;

// --------------------------------------------------

pub struct Resource {
	name: &'static str,

	static_resources: Option<Vec<Resource>>,
	pattern_resources: Option<Vec<Resource>>,
	wildcard_resource: Option<Box<Resource>>,

	request_receiver: Option<BoxedService>,
	request_passer: Option<BoxedService>,
	request_handler: Option<BoxedService>,

	// TODO: configs, state, redirect, parent
	is_subtree_handler: bool,
}

// -------------------------

impl Resource {
	#[inline]
	pub fn is_subtree_handler(&self) -> bool {
		self.is_subtree_handler
	}

	#[inline]
	pub fn can_handle_request(&self) -> bool {
		self.request_handler.is_some()
	}

	fn request_receiver(&self, mut req: Request) -> Result<Response, BoxedError> {
		let rs = req.extensions_mut().get_mut::<RoutingState>().unwrap();

		if rs.path_segments.has_remaining_segments() {
			if self.is_subtree_handler() {
				rs.subtree_handler_exists = true;
			}

			req = match self.request_passer(req) {
				Ok(mut res) => {
					if res.status() != StatusCode::NOT_FOUND
						|| !self.is_subtree_handler()
						|| !self.can_handle_request()
					{
						return Ok(res);
					}

					let Some(unused_request) = res.extensions_mut().remove::<UnusedRequest>() else {
						// Some middleware or handler set by the user has used the request and returned
						// NOT_FOUND.
						return Ok(res);
					};

					unused_request.into()
				}
				err => return err,
			}
		}

		return self.request_handler(req);
	}

	fn request_passer(&self, mut req: Request) -> Result<Response, BoxedError> {
		let rs = req.extensions_mut().get_mut::<RoutingState>().unwrap();

		// request_passer wouldn't be called if there wasn't any segment left. So, we can unwrap safely.
		let next_path_segment = rs.path_segments.next().unwrap();

		let some_next_resource = 'some_next_resource: {
			if let Some(next_resource) = self.static_resources.as_ref().and_then(|static_resources| {
				match static_resources
					.binary_search_by(|resource| resource.name.cmp(next_path_segment.as_str()))
				{
					Ok(i) => Some(&static_resources[i]),
					Err(_) => None,
				}
			}) {
				break 'some_next_resource Some(next_resource);
			}

			None
		};

		if let Some(next_resource) = some_next_resource {
			match next_resource.request_receiver(req) {
				Ok(mut res) => {
					if res.status() != StatusCode::NOT_FOUND
						|| !self.is_subtree_handler()
						|| !self.can_handle_request()
					{
						return Ok(res);
					}

					let Some(unused_request) = res.extensions_mut().get_mut::<UnusedRequest>() else {
						// Some middleware or a service set by user has used the request and returned NOT_FOUND.
						return Ok(res);
					};

					// Unwrap safety: returned request is guaranteed to have a RoutingState.
					let rs = unused_request
						.as_mut()
						.extensions_mut()
						.get_mut::<RoutingState>()
						.unwrap();
					rs.path_segments.revert_to_segment(next_path_segment);

					return Ok(res);
				}
				err => return err,
			}
		}

		let mut response = Response::default();
		*response.status_mut() = StatusCode::NOT_FOUND;

		if rs.subtree_handler_exists {
			rs.path_segments.revert_to_segment(next_path_segment);
			response.extensions_mut().insert(req);
		}

		Ok(response)
	}

	#[inline]
	fn request_handler(&self, req: Request) -> Result<Response, BoxedError> {
		todo!()
	}
}
