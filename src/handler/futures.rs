use std::{
	convert::Infallible,
	future::Future,
	pin::{pin, Pin},
	task::{Context, Poll},
};

use pin_project::pin_project;

// -------------------------

use crate::{
	request::Request,
	response::Response,
	routing::{RoutingState, StatusCode, UnusedRequest},
};

use super::{
	request_handlers::{misdirected_request_handler, request_passer, request_receiver},
	Handler,
};

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

#[pin_project]
pub struct ResultToResponseFuture<F>(#[pin] F);

impl<F> From<F> for ResultToResponseFuture<F> {
	fn from(inner: F) -> Self {
		Self(inner)
	}
}

impl<F, R> Future for ResultToResponseFuture<F>
where
	F: Future<Output = Result<R, Infallible>>,
{
	type Output = R;

	#[inline]
	fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
		self.project().0.poll(cx).map(|output| output.unwrap())
	}
}

// --------------------------------------------------------------------------------

#[pin_project]
pub struct ResponseToResultFuture<F>(#[pin] F);

impl<F> From<F> for ResponseToResultFuture<F> {
	fn from(inner: F) -> Self {
		Self(inner)
	}
}

impl<F, R> Future for ResponseToResultFuture<F>
where
	F: Future<Output = R>,
{
	type Output = Result<R, Infallible>;

	#[inline]
	fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
		self.project().0.poll(cx).map(|output| Ok(output))
	}
}

// --------------------------------------------------------------------------------

pub(crate) struct DefaultResponseFuture;

impl DefaultResponseFuture {
	pub(crate) fn new() -> Self {
		Self
	}
}

impl Future for DefaultResponseFuture {
	type Output = Response;

	fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
		Poll::Ready(Response::default())
	}
}

// --------------------------------------------------------------------------------

#[pin_project]
pub(crate) struct RequestReceiverFuture(Option<Request>);

impl From<Request> for RequestReceiverFuture {
	fn from(request: Request) -> Self {
		Self(Some(request))
	}
}

impl Future for RequestReceiverFuture {
	type Output = Response;

	fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
		let self_projection = self.project();

		let mut request = self_projection.0.take().unwrap();
		let mut routing_state = request.extensions_mut().remove::<RoutingState>().unwrap();
		let current_resource = routing_state.current_resource.clone().unwrap();

		if routing_state
			.path_traversal
			.has_remaining_segments(request.uri().path())
		{
			if current_resource.is_subtree_handler() {
				routing_state.subtree_handler_exists = true;
			}

			request.extensions_mut().insert(routing_state);

			let mut response = match current_resource.request_passer.as_ref() {
				Some(request_passer) => {
					if let Poll::Ready(response) = pin!(request_passer.handle(request)).poll(cx) {
						response
					} else {
						return Poll::Pending;
					}
				}
				None => {
					if let Poll::Ready(response) = pin!(request_passer(request)).poll(cx) {
						response
					} else {
						return Poll::Pending;
					}
				}
			};

			if response.status() != StatusCode::NOT_FOUND
				|| !current_resource.is_subtree_handler()
				|| !current_resource.can_handle_request()
			{
				return Poll::Ready(response);
			}

			let Some(unused_request) = response.extensions_mut().remove::<UnusedRequest>() else {
				return Poll::Ready(response);
			};

			request = unused_request.into_request()
		} else {
			request.extensions_mut().insert(routing_state);
		}

		if let Some(request_handler) = current_resource.request_handler.as_ref() {
			return pin!(request_handler.handle(request)).poll(cx);
		}

		if current_resource.method_handlers.is_empty() {
			return pin!(misdirected_request_handler(request)).poll(cx);
		}

		pin!(current_resource.method_handlers.handle(request)).poll(cx)
	}
}

// --------------------------------------------------------------------------------

#[pin_project]
pub struct RequestPasserFuture(Option<Request>);

impl From<Request> for RequestPasserFuture {
	fn from(request: Request) -> Self {
		Self(Some(request))
	}
}

impl Future for RequestPasserFuture {
	type Output = Response;

	fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
		let self_projection = self.project();

		let mut request = self_projection.0.take().unwrap();
		let mut routing_state = request.extensions_mut().remove::<RoutingState>().unwrap();
		let current_resource = routing_state.current_resource.take().unwrap();

		let (some_next_resource, next_segment_index) = 'some_next_resource: {
			let (next_segment, next_segment_index) = routing_state
				.path_traversal
				.next_segment(request.uri().path())
				.unwrap();

			if let Some(next_resource) = current_resource
				.static_resources
				.iter()
				.find(|resource| resource.pattern.is_match(next_segment))
			{
				break 'some_next_resource (Some(next_resource), next_segment_index);
			}

			if let Some(next_resource) = current_resource
				.regex_resources
				.iter()
				.find(|resource| resource.pattern.is_match(next_segment))
			{
				break 'some_next_resource (Some(next_resource), next_segment_index);
			}

			(
				current_resource.wildcard_resource.as_deref(),
				next_segment_index,
			)
		};

		if let Some(next_resource) = some_next_resource {
			routing_state
				.current_resource
				.replace(next_resource.clone());
			request.extensions_mut().insert(routing_state);

			let mut response = match next_resource.request_receiver.as_ref() {
				Some(request_receiver) => {
					if let Poll::Ready(response) = pin!(request_receiver.handle(request)).poll(cx) {
						response
					} else {
						return Poll::Pending;
					}
				}
				None => {
					if let Poll::Ready(response) = pin!(request_receiver(request)).poll(cx) {
						response
					} else {
						return Poll::Pending;
					}
				}
			};

			let Some(unused_request) = response.extensions_mut().get_mut::<UnusedRequest>() else {
				return Poll::Ready(response);
			};

			let req = unused_request.as_mut();

			let routing_state = req.extensions_mut().get_mut::<RoutingState>().unwrap();
			routing_state.current_resource.replace(current_resource);
			routing_state
				.path_traversal
				.revert_to_segment(next_segment_index);

			return Poll::Ready(response);
		}

		request.extensions_mut().insert(routing_state);

		pin!(misdirected_request_handler(request)).poll(cx)
	}
}
