//! Request handler kinds.

// ----------

use std::str::FromStr;

use argan_core::{body::Body, response::Response, BoxedFuture};
use http::Method;

use crate::response::BoxedErrorResponse;

use super::{BoxedHandler, FinalHandler, Handler, IntoHandler};

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

mod private {
	use crate::common::IntoArray;

	use super::*;

	#[allow(private_interfaces)]
	pub enum HandlerKind {
		Method(Method, BoxedHandler),
		WildcardMethod(Option<BoxedHandler>),
		MistargetedRequest(BoxedHandler),
	}

	impl IntoArray<HandlerKind, 1> for HandlerKind {
		fn into_array(self) -> [HandlerKind; 1] {
			[self]
		}
	}
}

pub(crate) use private::HandlerKind;

// --------------------------------------------------

macro_rules! handler_kind_by_method {
	($name:ident, $http_method:path, #[$comment:meta]) => {
		#[$comment]
		#[allow(non_camel_case_types)]
		pub struct $name;

		impl $name {
			pub fn to<H, Mark>(self, handler: H) -> HandlerKind
			where
				H: IntoHandler<Mark, Body>,
				H::Handler: Handler<
						Response = Response,
						Error = BoxedErrorResponse,
						Future = BoxedFuture<Result<Response, BoxedErrorResponse>>,
					> + Clone
					+ Send
					+ Sync
					+ 'static,
			{
				let final_handler = handler.into_handler();

				HandlerKind::Method($http_method, final_handler.into_boxed_handler())
			}
		}
	};
}

handler_kind_by_method!(
	_get,
	Method::GET,
	#[doc = "Setter type to pass the handler as a `GET` handler."]
);

handler_kind_by_method!(
	_head,
	Method::HEAD,
	#[doc = "Setter type to pass the handler as a `HEAD` handler."]
);

handler_kind_by_method!(
	_post,
	Method::POST,
	#[doc = "Setter type to pass the handler as a `POST` handler."]
);

handler_kind_by_method!(
	_put,
	Method::PUT,
	#[doc = "Setter type to pass the handler as a `PUT` handler."]
);

handler_kind_by_method!(
	_patch,
	Method::PATCH,
	#[doc = "Setter type to pass the handler as a `PATCH` handler"]
);

handler_kind_by_method!(
	_delete,
	Method::DELETE,
	#[doc = "Setter type to pass the handler as a `DELETE` handler"]
);

handler_kind_by_method!(
	_options,
	Method::OPTIONS,
	#[doc = "Setter type to pass the handler as an `OPTIONS` handler"]
);

handler_kind_by_method!(
	_connect,
	Method::CONNECT,
	#[doc = "Setter type to pass the handler as a `CONNECT` handler"]
);

handler_kind_by_method!(
	_trace,
	Method::TRACE,
	#[doc = "Setter type to pass the handler as a `TRACE` handler"]
);

/// Setter type to pass the handler as a *custom HTTP method* handler.
#[allow(non_camel_case_types)]
pub struct _method<M>(pub M);

impl<M: AsRef<str>> _method<M> {
	pub fn to<H, Mark>(self, handler: H) -> HandlerKind
	where
		H: IntoHandler<Mark, Body>,
		H::Handler: Handler<
				Response = Response,
				Error = BoxedErrorResponse,
				Future = BoxedFuture<Result<Response, BoxedErrorResponse>>,
			> + Clone
			+ Send
			+ Sync
			+ 'static,
	{
		let method = Method::from_str(self.0.as_ref())
			.expect("HTTP method should be a valid token [RFC 9110, 5.6.2 Tokens]");

		let final_handler = handler.into_handler();

		HandlerKind::Method(method, final_handler.into_boxed_handler())
	}
}

// --------------------------------------------------
// _wildcard_method

/// Setter type to pass the handler as a *wildcard method* handler.
///
/// A *wildcard method* handler is called when there is no dedicated handler
/// for the request's method.
#[allow(non_camel_case_types)]
pub struct _wildcard_method;

impl _wildcard_method {
	/// When there is a *method* handler or more, the default *wildcard method* handler responds
	/// with an `"Allow"` header listing the supported methods. To set the custom *wildcard method*
	/// handler, pass `Some(handler)`. To turn this functionality off, pass `None`.
	pub fn to<H, Mark>(self, some_handler: Option<H>) -> HandlerKind
	where
		H: IntoHandler<Mark, Body>,
		H::Handler: Handler<
				Response = Response,
				Error = BoxedErrorResponse,
				Future = BoxedFuture<Result<Response, BoxedErrorResponse>>,
			> + Clone
			+ Send
			+ Sync
			+ 'static,
	{
		let some_final_handler = some_handler.map(|handler| {
			let final_handler = handler.into_handler();

			final_handler.into_boxed_handler()
		});

		HandlerKind::WildcardMethod(some_final_handler)
	}
}

// --------------------------------------------------
// _wildcard_method

/// Setter type to pass the handler as a *mistargeted request* handler.
///
/// A *mistargeted request* handler is called when there is no resource matching
/// the request's path.
#[allow(non_camel_case_types)]
pub struct _mistargeted_request;

impl _mistargeted_request {
	pub fn to<H, Mark>(self, handler: H) -> HandlerKind
	where
		H: IntoHandler<Mark, Body>,
		H::Handler: Handler<
				Response = Response,
				Error = BoxedErrorResponse,
				Future = BoxedFuture<Result<Response, BoxedErrorResponse>>,
			> + Clone
			+ Send
			+ Sync
			+ 'static,
	{
		let final_handler = handler.into_handler();

		HandlerKind::MistargetedRequest(final_handler.into_boxed_handler())
	}
}

// --------------------------------------------------------------------------------
