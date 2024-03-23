use std::str::FromStr;

use http::Method;

use crate::{
	body::Body,
	common::{BoxedError, IntoArray},
	middleware::{IntoResponseResultAdapter, ResponseResultFutureBoxer},
	response::{BoxedErrorResponse, IntoResponse},
};

use super::{BoxedHandler, FinalHandler, Handler, IntoHandler};

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

mod private {
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
			H::Handler: Handler + Clone + Send + Sync + 'static,
			<H::Handler as Handler>::Response: IntoResponse,
			<H::Handler as Handler>::Error: Into<BoxedErrorResponse>,
			<H::Handler as Handler>::Future: Send,
		{
			let final_handler =
				ResponseResultFutureBoxer::wrap(IntoResponseResultAdapter::wrap(handler.into_handler()));

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
	H::Handler: Handler + Clone + Send + Sync + 'static,
	<H::Handler as Handler>::Response: IntoResponse,
	<H::Handler as Handler>::Error: Into<BoxedErrorResponse>,
	<H::Handler as Handler>::Future: Send,
{
	let method = Method::from_str(method.as_ref())
		.expect("HTTP method should be a valid token [RFC 9110, 5.6.2 Tokens]");

	let final_handler =
		ResponseResultFutureBoxer::wrap(IntoResponseResultAdapter::wrap(handler.into_handler()));

	HandlerKind::Method(method, final_handler.into_boxed_handler())
}

pub fn _wildcard_method<H, Mark>(some_handler: Option<H>) -> HandlerKind
where
	H: IntoHandler<Mark, Body>,
	H::Handler: Handler + Clone + Send + Sync + 'static,
	<H::Handler as Handler>::Response: IntoResponse,
	<H::Handler as Handler>::Error: Into<BoxedErrorResponse>,
	<H::Handler as Handler>::Future: Send,
{
	let some_final_handler = some_handler.map(|handler| {
		let handler = handler.into_handler();

		ResponseResultFutureBoxer::wrap(IntoResponseResultAdapter::wrap(handler)).into_boxed_handler()
	});

	HandlerKind::WildcardMethod(some_final_handler)
}

pub fn _mistargeted_request<H, Mark>(handler: H) -> HandlerKind
where
	H: IntoHandler<Mark, Body>,
	H::Handler: Handler + Clone + Send + Sync + 'static,
	<H::Handler as Handler>::Response: IntoResponse,
	<H::Handler as Handler>::Error: Into<BoxedErrorResponse>,
	<H::Handler as Handler>::Future: Send,
{
	let final_handler =
		ResponseResultFutureBoxer::wrap(IntoResponseResultAdapter::wrap(handler.into_handler()));

	HandlerKind::MistargetedRequest(final_handler.into_boxed_handler())
}

// --------------------------------------------------------------------------------
