use std::str::FromStr;

use argan_core::{body::Body, response::Response, BoxedFuture};
use http::Method;

use crate::response::{BoxedErrorResponse, IntoResponse};

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
	($func:ident, $http_method:path) => {
		pub fn $func<H, Mark>(handler: H) -> HandlerKind
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
	};
}

handler_kind_by_method!(_get, Method::GET);
handler_kind_by_method!(_head, Method::HEAD);
handler_kind_by_method!(_post, Method::POST);
handler_kind_by_method!(_put, Method::PUT);
handler_kind_by_method!(_patch, Method::PATCH);
handler_kind_by_method!(_delete, Method::DELETE);
handler_kind_by_method!(_options, Method::OPTIONS);
handler_kind_by_method!(_connect, Method::CONNECT);
handler_kind_by_method!(_trace, Method::TRACE);

pub fn _method<M, H, Mark>(method: M, handler: H) -> HandlerKind
where
	M: AsRef<str>,
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
	let method = Method::from_str(method.as_ref())
		.expect("HTTP method should be a valid token [RFC 9110, 5.6.2 Tokens]");

	let final_handler = handler.into_handler();

	HandlerKind::Method(method, final_handler.into_boxed_handler())
}

pub fn _wildcard_method<H, Mark>(some_handler: Option<H>) -> HandlerKind
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

pub fn _mistargeted_request<H, Mark>(handler: H) -> HandlerKind
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

// --------------------------------------------------------------------------------
