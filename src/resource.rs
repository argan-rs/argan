use hyper::StatusCode;

use super::{
	body::Incoming,
	handler::{request_handler::*, *},
	matcher::Matcher,
	request::Request,
	response::Response,
	routing::{RoutingState, UnusedRequest},
};

use super::utils::*;

// --------------------------------------------------

pub struct Resource {
	name: &'static str,
	matcher: Matcher,

	static_resources: Option<Vec<Resource>>,
	pattern_resources: Option<Vec<Resource>>,
	wildcard_resource: Option<Box<Resource>>,

	request_receiver: Option<BoxedHandler<Incoming>>,
	request_passer: Option<BoxedHandler<Incoming>>,
	request_handler: Option<BoxedHandler<Incoming>>,

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
}

fn request_receiver(mut request: Request) -> BoxedFuture<Result<Response, BoxedError>> {
	Box::pin(async move {
		let mut rs = request.extensions_mut().get_mut::<RoutingState>().unwrap();
		let cr = rs.current_resource.unwrap();

		if rs.path_segments.has_remaining_segments() {
			if cr.is_subtree_handler() {
				rs.subtree_handler_exists = true;
			}

			let result = match cr.request_passer.as_ref() {
				Some(request_passer) => request_passer.clone_boxed().call(request).await,
				None => request_passer(request).await,
			};

			let Ok(mut response) = result else {
				return result;
			};

			if response.status() != StatusCode::NOT_FOUND
				|| !cr.is_subtree_handler()
				|| !cr.can_handle_request()
			{
				return Ok(response);
			}

			let Some(unused_request) = response.extensions_mut().remove::<UnusedRequest>() else {
				return Ok(response);
			};

			request = unused_request.into_request()
		}

		if let Some(mut request_handler) = cr.request_handler.as_ref().map(|rh| rh.clone_boxed()) {
			request_handler.call(request).await
		} else {
			misdirected_request_handler(request).await
		}
	})
}

async fn request_passer(mut request: Request) -> Result<Response, BoxedError> {
	let rs = request.extensions_mut().get_mut::<RoutingState>().unwrap();
	let cr = rs.current_resource.unwrap();
	let next_path_segment = rs.path_segments.next().unwrap();

	let some_next_resource = 'some_next_resource: {
		if let Some(next_resource) = cr.static_resources.as_ref().and_then(|static_resources| {
			static_resources
				.iter()
				.find(|resource| resource.matcher.is_match(next_path_segment.as_str()))
		}) {
			break 'some_next_resource Some(next_resource);
		}

		if let Some(next_resource) = cr.pattern_resources.as_ref().and_then(|regex_resources| {
			regex_resources
				.iter()
				.find(|resource| resource.matcher.is_match(next_path_segment.as_str()))
		}) {
			break 'some_next_resource Some(next_resource);
		}

		cr.wildcard_resource.as_deref()
	};

	if let Some(next_resource) = some_next_resource {
		rs.current_resource.replace(next_resource);

		let result = match next_resource.request_receiver.as_ref() {
			Some(request_receiver) => request_receiver.clone_boxed().call(request).await,
			None => request_receiver(request).await,
		};

		let Ok(mut response) = result else {
			return result;
		};

		let Some(unused_request) = response.extensions_mut().get_mut::<UnusedRequest>() else {
			return Ok(response);
		};

		let req = unused_request.as_mut();

		let rs = req.extensions_mut().get_mut::<RoutingState>().unwrap();
		rs.current_resource.replace(cr);
		rs.path_segments.revert_to_segment(next_path_segment);

		return Ok(response);
	}

	misdirected_request_handler(request).await
}

async fn request_handler(_req: Request) -> Result<Response, BoxedError> {
	todo!()
}
