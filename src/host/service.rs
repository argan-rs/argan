use std::{convert::Infallible, future::ready, net::SocketAddr, sync::Arc};

use argan_core::{body::HttpBody, BoxedError, BoxedFuture};
use bytes::Bytes;
use http::StatusCode;
use hyper::service::Service;

use crate::{
	common::{header_utils::host_header_value, marker::Sealed, CloneWithPeerAddr},
	handler::Args,
	request::{
		routing::{RouteTraversal, RoutingState},
		Request, RequestContext, RequestContextProperties,
	},
	resource::FinalResource,
	response::{BoxedErrorResponse, InfallibleResponseFuture, IntoResponse, Response},
};

#[cfg(feature = "regex")]
use crate::pattern::ParamsList;

use super::*;

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

#[derive(Clone)]
pub(crate) struct FinalHost {
	pattern: Pattern,
	root_resource: FinalResource,
}

impl FinalHost {
	pub(super) fn new(pattern: Pattern, root_resource: FinalResource) -> Self {
		Self {
			pattern,
			root_resource,
		}
	}

	#[inline(always)]
	pub(crate) fn is_static_match(&self, text: &str) -> Option<bool> {
		self.pattern.is_static_match(text)
	}

	#[cfg(feature = "regex")]
	#[inline(always)]
	pub(crate) fn is_regex_match(&self, text: &str, params_list: &mut ParamsList) -> Option<bool> {
		self.pattern.is_regex_match(text, params_list)
	}

	#[inline(always)]
	pub(crate) fn handle<B>(
		&self,
		request_context: RequestContext<B>,
		args: Args,
	) -> BoxedFuture<Result<Response, BoxedErrorResponse>>
	where
		B: HttpBody<Data = Bytes> + Send + Sync + 'static,
		B::Error: Into<BoxedError>,
	{
		self.root_resource.handle(request_context, args)
	}
}

impl AsRef<FinalHost> for FinalHost {
	#[inline(always)]
	fn as_ref(&self) -> &FinalHost {
		self
	}
}

// --------------------------------------------------
// InnerHostService

struct InnerHostService<H = FinalHost> {
	host: H,

	#[cfg(feature = "peer-addr")]
	peer_addr: SocketAddr,
}

impl<H: AsRef<FinalHost>> InnerHostService<H> {
	#[inline(always)]
	fn new(final_host: H) -> Self {
		Self {
			host: final_host,

			#[cfg(feature = "peer-addr")]
			peer_addr: "0.0.0.0:0".parse().expect(SCOPE_VALIDITY),
		}
	}

	#[inline(always)]
	fn host_ref(&self) -> &FinalHost {
		self.host.as_ref()
	}
}

// ----------

impl Clone for InnerHostService<FinalHost> {
	#[inline(always)]
	fn clone(&self) -> Self {
		Self {
			host: self.host.clone(),

			#[cfg(feature = "peer-addr")]
			peer_addr: self.peer_addr,
		}
	}
}

impl Clone for InnerHostService<Arc<FinalHost>> {
	#[inline(always)]
	fn clone(&self) -> Self {
		Self {
			host: Arc::clone(&self.host),

			#[cfg(feature = "peer-addr")]
			peer_addr: self.peer_addr,
		}
	}
}

impl Clone for InnerHostService<&'static FinalHost> {
	#[inline(always)]
	fn clone(&self) -> Self {
		Self {
			host: self.host,

			#[cfg(feature = "peer-addr")]
			peer_addr: self.peer_addr,
		}
	}
}

// ----------

impl<B, H> Service<Request<B>> for InnerHostService<H>
where
	B: HttpBody<Data = Bytes> + Send + Sync + 'static,
	B::Error: Into<BoxedError>,
	H: AsRef<FinalHost>,
{
	type Response = Response;
	type Error = Infallible;
	type Future = InfallibleResponseFuture;

	fn call(&self, request: Request<B>) -> Self::Future {
		macro_rules! handle_unmatching_host {
			() => {
				InfallibleResponseFuture::from(Box::pin(ready(Ok(StatusCode::NOT_FOUND.into_response()))))
			};
		}

		let Ok(host) = host_header_value(&request) else {
			return handle_unmatching_host!();
		};

		#[allow(unused_mut)]
		let mut routing_state = RoutingState::new(RouteTraversal::for_route(request.uri().path()));
		let args = Args::new();

		if let Some(true) = self.host_ref().pattern.is_static_match(host) {
			let request_context = RequestContext::new(
				#[cfg(feature = "peer-addr")]
				self.peer_addr,
				request,
				routing_state,
				RequestContextProperties::default(),
			);

			return InfallibleResponseFuture::from(
				self.host_ref().root_resource.handle(request_context, args),
			);
		}

		#[cfg(feature = "regex")]
		if let Some(true) = self
			.host_ref()
			.pattern
			.is_regex_match(host, &mut routing_state.uri_params)
		{
			let request_context = RequestContext::new(
				#[cfg(feature = "peer-addr")]
				self.peer_addr,
				request,
				routing_state,
				RequestContextProperties::default(),
			);

			return InfallibleResponseFuture::from(
				self.host_ref().root_resource.handle(request_context, args),
			);
		}

		handle_unmatching_host!()
	}
}

// --------------------------------------------------
// HostService

/// A host service that can be used to handle requests.
///
/// Created by calling [`Host::into_service()`] on a `Host`.
#[derive(Clone)]
pub struct HostService(InnerHostService<FinalHost>);

impl HostService {
	#[inline(always)]
	pub(super) fn new(final_host: FinalHost) -> Self {
		Self(InnerHostService::new(final_host))
	}
}

impl<B> Service<Request<B>> for HostService
where
	B: HttpBody<Data = Bytes> + Send + Sync + 'static,
	B::Error: Into<BoxedError>,
{
	type Response = Response;
	type Error = Infallible;
	type Future = InfallibleResponseFuture;

	#[inline(always)]
	fn call(&self, request: Request<B>) -> Self::Future {
		self.0.call(request)
	}
}

impl CloneWithPeerAddr for HostService {
	#[inline(always)]
	fn clone_with_peer_addr(&self, _addr: SocketAddr) -> Self {
		Self(InnerHostService {
			host: self.0.host.clone(),
			#[cfg(feature = "peer-addr")]
			peer_addr: _addr,
		})
	}
}

impl Sealed for HostService {}

// --------------------------------------------------
// ArcHostservice

/// A host service that uses `Arc`.
///
/// Created by calling [`Host::into_arc_service()`] on a `Host`.
#[derive(Clone)]
pub struct ArcHostService(InnerHostService<Arc<FinalHost>>);

impl From<HostService> for ArcHostService {
	#[inline(always)]
	fn from(host_service: HostService) -> Self {
		let HostService(InnerHostService {
			host,

			#[cfg(feature = "peer-addr")]
			peer_addr,
		}) = host_service;

		Self(InnerHostService {
			host: Arc::new(host),

			#[cfg(feature = "peer-addr")]
			peer_addr,
		})
	}
}

impl<B> Service<Request<B>> for ArcHostService
where
	B: HttpBody<Data = Bytes> + Send + Sync + 'static,
	B::Error: Into<BoxedError>,
{
	type Response = Response;
	type Error = Infallible;
	type Future = InfallibleResponseFuture;

	#[inline(always)]
	fn call(&self, request: Request<B>) -> Self::Future {
		self.0.call(request)
	}
}

impl CloneWithPeerAddr for ArcHostService {
	fn clone_with_peer_addr(&self, _addr: SocketAddr) -> Self {
		Self(InnerHostService {
			host: Arc::clone(&self.0.host),
			#[cfg(feature = "peer-addr")]
			peer_addr: _addr,
		})
	}
}

impl Sealed for ArcHostService {}

// --------------------------------------------------
// LeakedHostService

/// A host service that uses leaked `&'static`.
///
/// Created by calling [`Host::into_leaked_service()`] on a `Host`.
#[derive(Clone)]
pub struct LeakedHostService(InnerHostService<&'static FinalHost>);

impl From<HostService> for LeakedHostService {
	#[inline(always)]
	fn from(host_service: HostService) -> Self {
		let HostService(InnerHostService {
			host,

			#[cfg(feature = "peer-addr")]
			peer_addr,
		}) = host_service;

		let final_host_ref = Box::leak(Box::new(host));

		Self(InnerHostService {
			host: final_host_ref,

			#[cfg(feature = "peer-addr")]
			peer_addr,
		})
	}
}

impl<B> Service<Request<B>> for LeakedHostService
where
	B: HttpBody<Data = Bytes> + Send + Sync + 'static,
	B::Error: Into<BoxedError>,
{
	type Response = Response;
	type Error = Infallible;
	type Future = InfallibleResponseFuture;

	#[inline(always)]
	fn call(&self, request: Request<B>) -> Self::Future {
		self.0.call(request)
	}
}

impl CloneWithPeerAddr for LeakedHostService {
	fn clone_with_peer_addr(&self, _addr: SocketAddr) -> Self {
		Self(InnerHostService {
			host: self.0.host,
			#[cfg(feature = "peer-addr")]
			peer_addr: _addr,
		})
	}
}

impl Sealed for LeakedHostService {}

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

#[cfg(all(test, feature = "full"))]
mod test {
	use crate::common::test_helpers::{
		new_root, test_service, Case, DataKind, Rx_1_1, Rx_2_0, Wl_3_0,
	};

	use super::*;

	// --------------------------------------------------------------------------------
	// --------------------------------------------------------------------------------

	#[tokio::test]
	async fn static_host_service() {
		// -------------------------
		// http://example.com

		//	http://example.com/	->	/st_0_0	->	/{wl_1_0}	->	/{rx_2_0:p_0}/
		//										|						|	->	/{rx_2_1:p_1}-abc	->	/{wl_3_0}
		//										|
		//										|	->	/{rx_1_1:p_0}-abc/	->	/st_2_0
		//																						|	->	/st_2_1

		let cases = [
			Case {
				name: "root",
				method: "GET",
				host: "http://example.com",
				path: "",
				some_content_type: Some(mime::TEXT_PLAIN_UTF_8),
				some_redirect_location: None,
				data_kind: DataKind::String("Hello, World!".to_string()),
			},
			Case {
				name: "st_0_0",
				method: "GET",
				host: "http://example.com",
				path: "/st_0_0",
				some_content_type: None,
				some_redirect_location: None,
				data_kind: DataKind::None,
			},
			Case {
				name: "rx_2_0",
				method: "GET",
				host: "http://example.com",
				path: "/st_0_0/42/p_0",
				some_content_type: None,
				some_redirect_location: Some("/st_0_0/42/p_0/"),
				data_kind: DataKind::None,
			},
			Case {
				name: "rx_2_0",
				method: "GET",
				host: "http://example.com",
				path: "/st_0_0/42/p_0/",
				some_content_type: Some(mime::APPLICATION_JSON),
				some_redirect_location: None,
				data_kind: DataKind::Rx_2_0(Rx_2_0 {
					sub: None,
					wl_1_0: 42,
					rx_2_0: "p_0".to_string(),
				}),
			},
			Case {
				name: "wl_3_0",
				method: "POST",
				host: "http://example.com",
				path: "/st_0_0/42/p_1-abc/true/",
				some_content_type: None,
				some_redirect_location: Some("/st_0_0/42/p_1-abc/true"),
				data_kind: DataKind::None,
			},
			Case {
				name: "wl_3_0",
				method: "POST",
				host: "http://example.com",
				path: "/st_0_0/42/p_1-abc/true",
				some_content_type: Some(mime::APPLICATION_JSON),
				some_redirect_location: None,
				data_kind: DataKind::Wl_3_0(Wl_3_0 {
					sub: None,
					wl_1_0: 42,
					rx_2_1: "p_1".to_string(),
					wl_3_0: true,
				}),
			},
			Case {
				name: "st_2_0",
				method: "GET",
				host: "http://example.com",
				path: "/st_0_0/p_0-abc/st_2_0",
				some_content_type: Some(mime::APPLICATION_JSON),
				some_redirect_location: None,
				data_kind: DataKind::Rx_1_1(Rx_1_1 {
					sub: None,
					rx_1_1: "p_0".to_string(),
				}),
			},
			Case {
				name: "rx_1_1",
				method: "GET",
				host: "http://example.com",
				path: "/st_0_0/p_0-abc",
				some_content_type: None,
				some_redirect_location: Some("/st_0_0/p_0-abc/"),
				data_kind: DataKind::None,
			},
			Case {
				name: "rx_1_1",
				method: "PUT",
				host: "http://example.com",
				path: "/st_0_0/p_0-abc/",
				some_content_type: Some(mime::APPLICATION_JSON),
				some_redirect_location: None,
				data_kind: DataKind::Rx_1_1(Rx_1_1 {
					sub: None,
					rx_1_1: "p_0".to_string(),
				}),
			},
			Case {
				name: "st_2_1",
				method: "GET",
				host: "http://example.com",
				path: "/st_0_0/p_0-abc/st_2_1",
				some_content_type: Some(mime::TEXT_PLAIN_UTF_8),
				some_redirect_location: None,
				data_kind: DataKind::String("Hello, World!".to_string()),
			},
		];

		let service = Host::new("http://example.com", new_root()).into_service();
		test_service(service, &cases[..]).await;
	}

	#[tokio::test]
	async fn regex_host_service() {
		// -------------------------
		//	http://{sub}.example.com

		//	http://{sub}.example.com/	->	/st_0_0	->	/{wl_1_0}	->	/{rx_2_0:p_0}/
		//													|						|	->	/{rx_2_1:p_1}-abc	->	/{wl_3_0}
		//													|
		//													|	->	/{rx_1_1:p_0}-abc/	->	/st_2_0
		//																									|	->	/st_2_1

		let cases = [
			Case {
				name: "root",
				method: "GET",
				host: "http://www.example.com",
				path: "",
				some_content_type: Some(mime::APPLICATION_JSON),
				some_redirect_location: None,
				data_kind: DataKind::String("www".to_string()),
			},
			Case {
				name: "st_0_0",
				method: "GET",
				host: "http://www.example.com",
				path: "/st_0_0",
				some_content_type: None,
				some_redirect_location: None,
				data_kind: DataKind::None,
			},
			Case {
				name: "rx_2_0",
				method: "GET",
				host: "http://www.example.com",
				path: "/st_0_0/42/p_0",
				some_content_type: None,
				some_redirect_location: Some("/st_0_0/42/p_0/"),
				data_kind: DataKind::None,
			},
			Case {
				name: "rx_2_0",
				method: "GET",
				host: "http://www.example.com",
				path: "/st_0_0/42/p_0/",
				some_content_type: Some(mime::APPLICATION_JSON),
				some_redirect_location: None,
				data_kind: DataKind::Rx_2_0(Rx_2_0 {
					sub: Some("www".to_string()),
					wl_1_0: 42,
					rx_2_0: "p_0".to_string(),
				}),
			},
			Case {
				name: "wl_3_0",
				method: "POST",
				host: "http://www.example.com",
				path: "/st_0_0/42/p_1-abc/true/",
				some_content_type: None,
				some_redirect_location: Some("/st_0_0/42/p_1-abc/true"),
				data_kind: DataKind::None,
			},
			Case {
				name: "wl_3_0",
				method: "POST",
				host: "http://www.example.com",
				path: "/st_0_0/42/p_1-abc/true",
				some_content_type: Some(mime::APPLICATION_JSON),
				some_redirect_location: None,
				data_kind: DataKind::Wl_3_0(Wl_3_0 {
					sub: Some("www".to_string()),
					wl_1_0: 42,
					rx_2_1: "p_1".to_string(),
					wl_3_0: true,
				}),
			},
			Case {
				name: "st_2_0",
				method: "GET",
				host: "http://www.example.com",
				path: "/st_0_0/p_0-abc/st_2_0",
				some_content_type: Some(mime::APPLICATION_JSON),
				some_redirect_location: None,
				data_kind: DataKind::Rx_1_1(Rx_1_1 {
					sub: Some("www".to_string()),
					rx_1_1: "p_0".to_string(),
				}),
			},
			Case {
				name: "rx_1_1",
				method: "GET",
				host: "http://www.example.com",
				path: "/st_0_0/p_0-abc",
				some_content_type: None,
				some_redirect_location: Some("/st_0_0/p_0-abc/"),
				data_kind: DataKind::None,
			},
			Case {
				name: "rx_1_1",
				method: "PUT",
				host: "http://www.example.com",
				path: "/st_0_0/p_0-abc/",
				some_content_type: Some(mime::APPLICATION_JSON),
				some_redirect_location: None,
				data_kind: DataKind::Rx_1_1(Rx_1_1 {
					sub: Some("www".to_string()),
					rx_1_1: "p_0".to_string(),
				}),
			},
			Case {
				name: "st_2_1",
				method: "GET",
				host: "http://www.example.com",
				path: "/st_0_0/p_0-abc/st_2_1",
				some_content_type: Some(mime::TEXT_PLAIN_UTF_8),
				some_redirect_location: None,
				data_kind: DataKind::String("Hello, World!".to_string()),
			},
		];

		let service = Host::new("http://{sub}.example.com", new_root()).into_service();
		test_service(service, &cases).await;
	}
}
