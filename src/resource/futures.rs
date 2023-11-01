use std::{
	convert::Infallible,
	future::{Future, Ready},
	pin::{pin, Pin},
	sync::Arc,
	task::{Context, Poll},
};

use http::StatusCode;
use percent_encoding::percent_decode_str;
use pin_project_lite::pin_project;

use crate::{
	handler::{
		futures::ResponseToResultFuture, request_handlers::misdirected_request_handler, Handler,
	},
	request::Request,
	response::Response,
	routing::{RoutingState, UnusedRequest},
	utils::BoxedFuture,
};

use super::service::{request_passer, request_receiver};

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

pin_project! {
	pub(crate) struct RequestReceiverFuture {
		some_request: Option<Request>
	}
}

impl From<Request> for RequestReceiverFuture {
	fn from(request: Request) -> Self {
		Self {
			some_request: Some(request),
		}
	}
}

impl Future for RequestReceiverFuture {
	type Output = Response;

	fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
		let self_projection = self.project();

		let mut request = self_projection.some_request.take().unwrap();
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

pin_project! {
	pub struct RequestPasserFuture {
		some_request: Option<Request>
	}
}

impl From<Request> for RequestPasserFuture {
	fn from(request: Request) -> Self {
		Self {
			some_request: Some(request),
		}
	}
}

impl Future for RequestPasserFuture {
	type Output = Response;

	fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
		let self_projection = self.project();

		let mut request = self_projection.some_request.take().unwrap();
		let mut routing_state = request.extensions_mut().remove::<RoutingState>().unwrap();
		let current_resource = routing_state.current_resource.take().unwrap();
		let mut path_params = std::mem::take(&mut routing_state.path_params);

		let (some_next_resource, next_segment_index) = 'some_next_resource: {
			let (next_segment, next_segment_index) = routing_state
				.path_traversal
				.next_segment(request.uri().path())
				.unwrap();

			if let Some(next_resource) =
				current_resource
					.static_resources
					.as_ref()
					.and_then(|resources| {
						resources.iter().find(
							// Static patterns keep percent encoded text. We may match them without
							// decoding the segment.
							|resource| resource.pattern.is_static_match(next_segment).unwrap(),
						)
					}) {
				break 'some_next_resource (Some(next_resource), next_segment_index);
			}

			let decoded_segment =
				Arc::<str>::from(percent_decode_str(next_segment).decode_utf8().unwrap());

			if let Some(next_resource) = current_resource
				.regex_resources
				.as_ref()
				.and_then(|resources| {
					resources.iter().find(|resource| {
						resource
							.pattern
							.is_regex_match(decoded_segment.clone(), &mut path_params)
							.unwrap()
					})
				}) {
				break 'some_next_resource (Some(next_resource), next_segment_index);
			}

			current_resource
				.wildcard_resource
				.as_ref()
				.is_some_and(|resource| {
					resource
						.pattern
						.is_wildcard_match(decoded_segment, &mut path_params)
						.unwrap()
				});

			(
				current_resource.wildcard_resource.as_deref(),
				next_segment_index,
			)
		};

		if let Some(next_resource) = some_next_resource {
			routing_state.path_params = path_params;
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

// --------------------------------------------------------------------------------

pin_project! {
	pub struct ResourceFuture {
		#[pin] inner: ResourceInnerFuture,
	}
}

impl From<ResourceInnerFuture> for ResourceFuture {
	fn from(inner: ResourceInnerFuture) -> Self {
		Self { inner }
	}
}

impl Future for ResourceFuture {
	type Output = Result<Response, Infallible>;

	#[inline]
	fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
		self.project().inner.poll(cx)
	}
}

// ----------

pin_project! {
	#[project = ResourceInnerFutureProjection]
	pub(crate) enum ResourceInnerFuture {
		Boxed { #[pin] boxed: ResponseToResultFuture<BoxedFuture<Response>> },
		Unboxed { #[pin] unboxed: ResponseToResultFuture<RequestReceiverFuture> },
		Ready { #[pin] ready: ResponseToResultFuture<Ready<Response>> },
	}
}

impl From<BoxedFuture<Response>> for ResourceInnerFuture {
	#[inline]
	fn from(boxed_future: BoxedFuture<Response>) -> Self {
		Self::Boxed {
			boxed: ResponseToResultFuture::from(boxed_future),
		}
	}
}

impl From<RequestReceiverFuture> for ResourceInnerFuture {
	#[inline]
	fn from(request_receiver_future: RequestReceiverFuture) -> Self {
		Self::Unboxed {
			unboxed: ResponseToResultFuture::from(request_receiver_future),
		}
	}
}

impl From<Ready<Response>> for ResourceInnerFuture {
	#[inline]
	fn from(ready_future: Ready<Response>) -> Self {
		Self::Ready {
			ready: ResponseToResultFuture::from(ready_future),
		}
	}
}

impl Future for ResourceInnerFuture {
	type Output = Result<Response, Infallible>;

	#[inline]
	fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
		match self.project() {
			ResourceInnerFutureProjection::Boxed { boxed } => boxed.poll(cx),
			ResourceInnerFutureProjection::Unboxed { unboxed } => unboxed.poll(cx),
			ResourceInnerFutureProjection::Ready { ready } => ready.poll(cx),
		}
	}
}

// --------------------------------------------------
