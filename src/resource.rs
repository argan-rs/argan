use hyper::StatusCode;

use crate::pattern::Pattern;

use super::{
	body::Incoming,
	handler::{request_handler::*, *},
	request::Request,
	response::Response,
	routing::{RoutingState, UnusedRequest},
};

use super::utils::*;

// --------------------------------------------------

pub struct Resource {
	pattern: Pattern,

	static_resources: Option<Vec<Resource>>,
	regex_resources: Option<Vec<Resource>>,
	wildcard_resource: Option<Box<Resource>>,

	request_receiver: Option<HandlerService<Incoming>>,
	request_passer: Option<HandlerService<Incoming>>,
	request_handler: Option<HandlerService<Incoming>>,

	handlers: Handlers<Incoming>,

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
		let mut routing_state = request.extensions_mut().get_mut::<RoutingState>().unwrap();
		let current_resource = routing_state.current_resource.unwrap();

		if routing_state.path_segments.has_remaining_segments() {
			if current_resource.is_subtree_handler() {
				routing_state.subtree_handler_exists = true;
			}

			let result = match current_resource.request_passer.as_ref() {
				Some(request_passer) => request_passer.clone().call(request).await,
				None => request_passer(request).await,
			};

			let Ok(mut response) = result else {
				return result;
			};

			if response.status() != StatusCode::NOT_FOUND
				|| !current_resource.is_subtree_handler()
				|| !current_resource.can_handle_request()
			{
				return Ok(response);
			}

			let Some(unused_request) = response
				.extensions_mut()
				.remove::<UnusedRequest<Incoming>>()
			else {
				return Ok(response);
			};

			request = unused_request.into_request()
		}

		if let Some(request_handler) = current_resource.request_handler.as_ref() {
			return request_handler.clone().call(request).await;
		}

		if current_resource.handlers.is_empty() {
			return misdirected_request_handler(request).await;
		}

		current_resource.handlers.handle(request).await
	})
}

async fn request_passer(mut request: Request) -> Result<Response, BoxedError> {
	let routing_state = request.extensions_mut().get_mut::<RoutingState>().unwrap();
	let current_resource = routing_state.current_resource.unwrap(); // ???
	let next_path_segment = routing_state.path_segments.next().unwrap();

	let some_next_resource = 'some_next_resource: {
		if let Some(next_resource) =
			current_resource
				.static_resources
				.as_ref()
				.and_then(|static_resources| {
					static_resources
						.iter()
						.find(|resource| resource.pattern.is_match(next_path_segment.as_str()))
				}) {
			break 'some_next_resource Some(next_resource);
		}

		if let Some(next_resource) =
			current_resource
				.regex_resources
				.as_ref()
				.and_then(|regex_resources| {
					regex_resources
						.iter()
						.find(|resource| resource.pattern.is_match(next_path_segment.as_str()))
				}) {
			break 'some_next_resource Some(next_resource);
		}

		current_resource.wildcard_resource.as_deref()
	};

	if let Some(next_resource) = some_next_resource {
		routing_state.current_resource.replace(next_resource);

		let result = match next_resource.request_receiver.as_ref() {
			Some(request_receiver) => request_receiver.clone().call(request).await,
			None => request_receiver(request).await,
		};

		let Ok(mut response) = result else {
			return result;
		};

		let Some(unused_request) = response
			.extensions_mut()
			.get_mut::<UnusedRequest<Incoming>>()
		else {
			return Ok(response);
		};

		let req = unused_request.as_mut();

		let routing_state = req.extensions_mut().get_mut::<RoutingState>().unwrap();
		routing_state.current_resource.replace(current_resource);
		routing_state
			.path_segments
			.revert_to_segment(next_path_segment);

		return Ok(response);
	}

	misdirected_request_handler(request).await
}

fn request_handler(mut request: Request) -> BoxedFuture<Result<Response, BoxedError>> {
	let routing_state = request.extensions_mut().get_mut::<RoutingState>().unwrap();
	let current_resource = routing_state.current_resource.unwrap(); // ???

	current_resource.handlers.handle(request)
}
