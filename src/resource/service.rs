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
	utils::{BoxedError, BoxedFuture, Uncloneable},
};

use super::futures::{
	RequestPasserFuture, RequestReceiverFuture, ResourceFuture, ResourceInnerFuture,
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

	// TODO: configs, redirect
	pub(super) is_subtree_handler: bool,
}

impl ResourceService {
	#[inline(always)]
	fn is_root(&self) -> bool {
		match self.pattern {
			Pattern::Static(ref pattern) => pattern.as_ref() == "/",
			_ => false,
		}
	}

	#[inline(always)]
	pub(super) fn is_subtree_handler(&self) -> bool {
		self.is_subtree_handler
	}

	#[inline(always)]
	pub(super) fn can_handle_request(&self) -> bool {
		!self.method_handlers.is_empty()
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
			self.is_root()
		} else if self.is_root() {
			// Resource is a root and the request's path always starts from root.
			true
		} else {
			let (next_segment, _) = route_traversal
				.next_segment(route)
				.expect("route should contain a path segment");

			// If pattern is static, we may match it without decoding the segment.
			// Static patterns keep percent-encoded string.
			if let Some(result) = self.pattern.is_static_match(next_segment) {
				result
			} else {
				let decoded_segment = Arc::<str>::from(
					percent_decode_str(next_segment)
						.decode_utf8()
						.expect("decoded segment should be a valid utf8 string"), // ???
				);

				if let Some(result) = self
					.pattern
					.is_regex_match(decoded_segment.clone(), &mut path_params)
				{
					result
				} else {
					self
						.pattern
						.is_wildcard_match(decoded_segment, &mut path_params)
						.expect("wildcard_resource must keep only a resource with a wilcard pattern")
				}
			}
		};

		let mut routing_state = RoutingState::new(route_traversal, self.clone());
		routing_state.path_params = path_params;
		request
			.extensions_mut()
			.insert(Uncloneable::from(routing_state));

		if matched {
			match self.request_receiver.as_ref() {
				Some(request_receiver) => {
					ResourceInnerFuture::from(request_receiver.handle(request)).into()
				}
				None => ResourceInnerFuture::from(request_receiver(request)).into(),
			}
		} else {
			ResourceInnerFuture::from(misdirected_request_handler(request)).into()
		}
	}
}

// --------------------------------------------------------------------------------

#[inline(always)]
pub(super) fn request_receiver(request: Request) -> RequestReceiverFuture {
	RequestReceiverFuture::from(request)
}

#[inline(always)]
pub(super) fn request_passer(request: Request) -> RequestPasserFuture {
	RequestPasserFuture::from(request)
}

#[inline(always)]
pub(super) fn request_handler(request: Request) -> BoxedFuture<Response> {
	let routing_state = request
		.extensions()
		.get::<Uncloneable<RoutingState>>()
		.expect("Uncloneable<RoutingState> should be inserted before request_handler is called")
		.as_ref()
		.expect("RoutingState should always exist in Uncloneable");

	let current_resource = routing_state.current_resource.clone().expect(
		"current resource should be set in the request_passer or the call method of the Service",
	);

	current_resource.method_handlers.handle(request)
}

#[cfg(test)]
mod test {
	use std::str::FromStr;

	use http::{header::CONTENT_TYPE, Method, StatusCode, Uri};
	use http_body_util::BodyExt;
	use serde::{Deserialize, Serialize};

	use crate::{
		body::{Bytes, Empty},
		data::Json,
		request::PathParam,
		resource::Resource,
	};

	use super::*;

	// --------------------------------------------------

	#[tokio::test]
	async fn resource_service() {
		let mut root = Resource::new("/");
		let handler = |_request: Request| async {};
		root
			.subresource_mut("/abc")
			.set_handler(Method::GET, handler);
		assert_eq!(root.subresource_mut("/abc").pattern(), "abc");
		assert!(root.subresource_mut("/abc").can_handle_request());

		let service = root.into_service();
		let static_resource = service.static_resources.as_ref().unwrap();
		assert_eq!(static_resource.len(), 1);
		assert_eq!(static_resource[0].pattern.to_string(), "abc");

		let request = Request::get("/abc").body(Empty::<Bytes>::new()).unwrap();
		let response = service.call(request).await.unwrap();
		assert_eq!(response.status(), StatusCode::OK);

		// --------------------------------------------------
		// --------------------------------------------------
		//		abc0_0 -> *abc1_0 -> $abc2_0:@(abc2_0)
		//					 |					-> $abc2_1:@cn(abc2_1)-cba -> *abc3_0
		//					 |
		//					 -> $abc1_1:@cn(abc1_1)-cba -> abc2_0

		let mut resource = Resource::new("/abc0_0");
		resource.set_handler(Method::GET, hello_world);

		resource.subresource_mut("/*abc1_0").set_handler(
			Method::PUT,
			|PathParam(wildcard): PathParam<String>| async move {
				println!("got param: {}", wildcard);

				wildcard
			},
		);

		resource
			.subresource_mut("/*abc1_0/$abc2_0:@(abc2_0)")
			.set_handler(
				Method::POST,
				|PathParam(path_values): PathParam<PathValues1_0_2_0>| async move {
					println!("got path values: {:?}", path_values);

					Json(path_values)
				},
			);

		#[derive(Debug, Serialize, Deserialize)]
		struct PathValues1_0_2_0 {
			abc1_0: String,
			abc2_0: Option<String>,
			abc3_0: Option<u64>,
		}

		resource
			.subresource_mut("/*abc1_0/$abc2_1:@cn(abc2_1)-cba/*abc3_0")
			.set_handler(
				Method::GET,
				|PathParam(path_values): PathParam<PathValues1_0_2_1_3_0>| async move {
					println!("got path values: {:?}", path_values);

					Json(path_values)
				},
			);

		#[derive(Debug, Serialize, Deserialize)]
		struct PathValues1_0_2_1_3_0 {
			abc1_0: Option<String>,
			abc2_1: String,
			abc3_0: u64,
		}

		resource
			.subresource_mut("/$abc1_1:@cn(abc1_1)-cba")
			.set_handler(
				Method::GET,
				|PathParam(value): PathParam<String>| async move {
					let vector = Vec::from(value);
					println!("got path values: {:?}", vector);

					vector
				},
			);

		resource
			.subresource_mut("/$abc1_1:@cn(abc1_1)-cba/abc2_0")
			.set_handler(Method::GET, hello_world);

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
		assert_eq!(
			response
				.headers()
				.get(CONTENT_TYPE)
				.unwrap()
				.to_str()
				.unwrap(),
			mime::TEXT_PLAIN_UTF_8,
		);

		let body = response.into_body().collect().await.unwrap().to_bytes();
		assert_eq!(body.as_ref(), "abc1_0".as_bytes());

		dbg!();

		let request = new_request("POST", "/abc0_0/abc1_0/abc2_0");
		let response = service.call(request).await.unwrap();
		assert_eq!(response.status(), StatusCode::OK);
		assert_eq!(
			response
				.headers()
				.get(CONTENT_TYPE)
				.unwrap()
				.to_str()
				.unwrap(),
			mime::APPLICATION_JSON,
		);

		let json_body = String::from_utf8(
			response
				.into_body()
				.collect()
				.await
				.unwrap()
				.to_bytes()
				.to_vec(),
		)
		.unwrap();
		assert_eq!(
			json_body,
			r#"{"abc1_0":"abc1_0","abc2_0":"abc2_0","abc3_0":null}"#
		);

		dbg!();

		let request = new_request("GET", "/abc0_0/abc1_1-cba");
		let response = service.call(request).await.unwrap();
		assert_eq!(response.status(), StatusCode::OK);
		assert_eq!(
			response
				.headers()
				.get(CONTENT_TYPE)
				.unwrap()
				.to_str()
				.unwrap(),
			mime::APPLICATION_OCTET_STREAM,
		);

		let vector_body = response
			.into_body()
			.collect()
			.await
			.unwrap()
			.to_bytes()
			.to_vec();
		assert_eq!(vector_body, b"abc1_1".to_vec());

		dbg!();

		let request = new_request("GET", "/abc0_0/abc1_1-cba/abc2_0");
		let response = service.call(request).await.unwrap();
		assert_eq!(response.status(), StatusCode::OK);

		dbg!();

		let request = new_request("GET", "/abc0_0/abc1_0-wildcard/abc2_1-cba/30");
		let response = service.call(request).await.unwrap();
		assert_eq!(response.status(), StatusCode::OK);
		assert_eq!(
			response
				.headers()
				.get(CONTENT_TYPE)
				.unwrap()
				.to_str()
				.unwrap(),
			mime::APPLICATION_JSON,
		);

		let json_body = String::from_utf8(
			response
				.into_body()
				.collect()
				.await
				.unwrap()
				.to_bytes()
				.to_vec(),
		)
		.unwrap();
		assert_eq!(
			json_body,
			r#"{"abc1_0":"abc1_0-wildcard","abc2_1":"abc2_1","abc3_0":30}"#
		);
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
