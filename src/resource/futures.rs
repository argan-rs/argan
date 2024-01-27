use std::{
	convert::Infallible,
	future::{Future, Ready},
	pin::{self, pin, Pin},
	sync::Arc,
	task::{Context, Poll},
};

use http::StatusCode;
use percent_encoding::percent_decode_str;
use pin_project::pin_project;

use crate::{
	common::{BoxedFuture, Uncloneable},
	handler::{
		futures::ResponseToResultFuture, request_handlers::handle_misdirected_request, Handler,
	},
	request::Request,
	response::Response,
	routing::{RoutingState, UnusedRequest},
};

use super::service::{request_passer, request_receiver};

// --------------------------------------------------------------------------------
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

		let mut request = self_projection
			.0
			.take()
			.expect("RequestReceiverFuture must be created from request");

		let mut routing_state = request
			.extensions_mut()
			.remove::<Uncloneable<RoutingState>>()
			.expect("Uncloneable<RoutingState> should be inserted before routing starts")
			.into_inner()
			.expect("RoutingState should always exist in Uncloneable");

		// We don't take out the current resource because both the request_passer and
		// request_handler need it.
		let current_resource = routing_state.current_resource.clone().expect(
			"current resource should be set in the request_passer or the call method of the Service",
		);

		if routing_state
			.path_traversal
			.has_remaining_segments(request.uri().path())
		{
			if current_resource.is_subtree_handler() {
				routing_state.subtree_handler_exists = true;
			}

			request
				.extensions_mut()
				.insert(Uncloneable::from(routing_state));

			let mut response = match current_resource.0.request_passer.as_ref() {
				Some(request_passer) => {
					// Current resource's request_passer was wrapped in middleware.
					if let Poll::Ready(response) = pin!(request_passer.handle(request)).poll(cx) {
						response
					} else {
						return Poll::Pending;
					}
				}
				None => {
					// General request_passer without any middleware.
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

			// At this point, no resource matched the request's path or the matched resource or
			// some middleware has decided to respond with the status 'Not Found'. In the former
			// case, the last request_passer in the subtree returns the request unused. We must
			// try to recover it.

			let some_uncloneable = response
				.extensions_mut()
				.remove::<Uncloneable<UnusedRequest>>();

			let Some(uncloneable) = some_uncloneable else {
				// The request has already been used by some matched resource or some middleware or
				// some other subtree handler below in the request's path.
				return Poll::Ready(response);
			};

			request = uncloneable
				.into_inner()
				.expect("unused request should always exist in Uncloneable")
				.into_request();
		} else {
			request
				.extensions_mut()
				.insert(Uncloneable::from(routing_state));
		}

		if let Some(request_handler) = current_resource.0.request_handler.as_ref() {
			// Current resource's request_handler was wrapped in middleware.
			return pin!(request_handler.handle(request)).poll(cx);
		}

		if !current_resource.can_handle_request() {
			return pin!(handle_misdirected_request(request)).poll(cx);
		}

		pin!(current_resource.0.method_handlers.handle(request)).poll(cx)
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

		let mut request = self_projection
			.0
			.take()
			.expect("RequestPasserFuture must be created from request");

		let mut routing_state = request
			.extensions_mut()
			.remove::<Uncloneable<RoutingState>>()
			.expect("Uncloneable<RoutingState> should be inserted before request_passer is called")
			.into_inner()
			.expect("RoutingState should always exist in Uncloneable");

		let current_resource = routing_state
			.current_resource
			.take()
			.expect("current resource should be set before creating a RequestPasserFuture");

		let mut path_params = std::mem::take(&mut routing_state.path_params);

		let (some_next_resource, next_segment_index) = 'some_next_resource: {
			let (next_segment, next_segment_index) = routing_state
				.path_traversal
				.next_segment(request.uri().path())
				.expect("RequestPasserFuture shouldn't be created when there is no next path segment");

			if let Some(next_resource) =
				current_resource
					.0
					.static_resources
					.as_ref()
					.and_then(|resources| {
						resources.iter().find(
							// Static patterns keep percent-encoded string. We may match them without
							// decoding the segment.
							|resource| {
								resource
									.0
									.pattern
									.is_static_match(next_segment)
									.expect("static_resources must keep only the resources with a static pattern")
							},
						)
					}) {
				break 'some_next_resource (Some(next_resource), next_segment_index);
			}

			let decoded_segment = Arc::<str>::from(
				percent_decode_str(next_segment)
					.decode_utf8()
					.expect("decoded segment should be a valid utf8 string"), // ???
			);

			if let Some(next_resource) =
				current_resource
					.0
					.regex_resources
					.as_ref()
					.and_then(|resources| {
						resources.iter().find(|resource| {
							resource
								.0
								.pattern
								.is_regex_match(decoded_segment.clone(), &mut path_params)
								.expect("regex_resources must keep only the resources with a regex pattern")
						})
					}) {
				break 'some_next_resource (Some(next_resource), next_segment_index);
			}

			let _ = current_resource
				.0
				.wildcard_resource
				.as_ref()
				.is_some_and(|resource| {
					resource
						.0
						.pattern
						.is_wildcard_match(decoded_segment, &mut path_params)
						.expect("wildcard_resource must keep only a resource with a wilcard pattern")
				});

			(
				current_resource.0.wildcard_resource.as_ref(),
				next_segment_index,
			)
		};

		if let Some(next_resource) = some_next_resource {
			routing_state.path_params = path_params;
			routing_state
				.current_resource
				.replace(next_resource.clone());

			request
				.extensions_mut()
				.insert(Uncloneable::from(routing_state));

			let mut response = match next_resource.0.request_receiver.as_ref() {
				Some(request_receiver) => {
					// Next resource's request_receiver was wrapped in middleware.
					if let Poll::Ready(response) = pin!(request_receiver.handle(request)).poll(cx) {
						response
					} else {
						return Poll::Pending;
					}
				}
				None => {
					// General request_receiver without any middleware.
					if let Poll::Ready(response) = pin!(request_receiver(request)).poll(cx) {
						response
					} else {
						return Poll::Pending;
					}
				}
			};

			// If the requested resource wasn't found in the subtree and there is a subtree handler
			// in the request's path, response contains the unused request. We must get it and recover
			// the current resource before returning to the request_receiver.
			let some_uncloneable = response
				.extensions_mut()
				.get_mut::<Uncloneable<UnusedRequest>>();

			let Some(uncloneable) = some_uncloneable else {
				return Poll::Ready(response);
			};

			let request = uncloneable
				.as_mut()
				.expect("unused request should always exist in Uncloneable")
				.as_mut();

			let routing_state = request
				.extensions_mut()
				.get_mut::<Uncloneable<RoutingState>>()
				.expect("unused request must always have Uncloneable<RoutingState>")
				.as_mut()
				.expect("RoutingState should always exist in Uncloneable");

			routing_state.current_resource.replace(current_resource);
			routing_state
				.path_traversal
				.revert_to_segment(next_segment_index);

			return Poll::Ready(response);
		}

		request
			.extensions_mut()
			.insert(Uncloneable::from(routing_state));

		// TODO: In the future, we may have a way to replace the 'misdirected_request_hanlder'
		// with a custom handler. Then we may consider returning the unused request in a response
		// from here.

		pin!(handle_misdirected_request(request)).poll(cx)
	}
}

// --------------------------------------------------------------------------------

#[pin_project]
pub struct ResourceFuture(#[pin] ResourceInnerFuture);

impl From<ResourceInnerFuture> for ResourceFuture {
	#[inline(always)]
	fn from(inner: ResourceInnerFuture) -> Self {
		Self(inner)
	}
}

impl Future for ResourceFuture {
	type Output = Result<Response, Infallible>;

	#[inline(always)]
	fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
		self.project().0.poll(cx)
	}
}

// ----------

#[pin_project(project = ResourceInnerFutureProjection)]
pub(crate) enum ResourceInnerFuture {
	Boxed {
		#[pin]
		boxed: ResponseToResultFuture<BoxedFuture<Response>>,
	},
	Unboxed {
		#[pin]
		unboxed: ResponseToResultFuture<RequestReceiverFuture>,
	},
	Ready {
		#[pin]
		ready: ResponseToResultFuture<Ready<Response>>,
	},
}

impl From<BoxedFuture<Response>> for ResourceInnerFuture {
	#[inline(always)]
	fn from(boxed_future: BoxedFuture<Response>) -> Self {
		Self::Boxed {
			boxed: ResponseToResultFuture::from(boxed_future),
		}
	}
}

impl From<RequestReceiverFuture> for ResourceInnerFuture {
	#[inline(always)]
	fn from(request_receiver_future: RequestReceiverFuture) -> Self {
		Self::Unboxed {
			unboxed: ResponseToResultFuture::from(request_receiver_future),
		}
	}
}

impl From<Ready<Response>> for ResourceInnerFuture {
	#[inline(always)]
	fn from(ready_future: Ready<Response>) -> Self {
		Self::Ready {
			ready: ResponseToResultFuture::from(ready_future),
		}
	}
}

impl Future for ResourceInnerFuture {
	type Output = Result<Response, Infallible>;

	#[inline(always)]
	fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
		match self.project() {
			ResourceInnerFutureProjection::Boxed { boxed } => boxed.poll(cx),
			ResourceInnerFutureProjection::Unboxed { unboxed } => unboxed.poll(cx),
			ResourceInnerFutureProjection::Ready { ready } => ready.poll(cx),
		}
	}
}

// --------------------------------------------------
