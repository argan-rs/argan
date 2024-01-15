use std::str::FromStr;

use http::Method;

use crate::{
	body::IncomingBody,
	middleware::{IntoResponseAdapter, ResponseFutureBoxer},
	response::IntoResponse, utils::IntoArray,
};

use super::{ArcHandler, Handler, IntoArcHandler, IntoHandler};

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

pub struct HandlerKind(pub(crate) Inner);

pub(crate) enum Inner {
	Method(Method, ArcHandler),
	AllMethods(ArcHandler),
	MisdirectedRequest(ArcHandler),
}

// ----------

impl IntoArray<1, HandlerKind> for HandlerKind {
	fn into_array(self) -> [HandlerKind; 1] {
	  [self]
	}
}

// --------------------------------------------------

macro_rules! handler_kind_by_method {
	($func:ident, $http_method:path) => {
		pub fn $func<H, M>(handler: H) -> HandlerKind
		where
			H: IntoHandler<M, IncomingBody>,
			H::Handler: Handler + Send + Sync + 'static,
			<H::Handler as Handler>::Response: IntoResponse,
		{
			let ready_handler =
				ResponseFutureBoxer::wrap(IntoResponseAdapter::wrap(handler.into_handler()));

			HandlerKind(Inner::Method(
				$http_method,
				ready_handler.into_arc_handler(),
			))
		}
	};
}

handler_kind_by_method!(get, Method::GET);
handler_kind_by_method!(head, Method::HEAD);
handler_kind_by_method!(post, Method::POST);
handler_kind_by_method!(put, Method::PUT);
handler_kind_by_method!(patch, Method::PATCH);
handler_kind_by_method!(delete, Method::DELETE);
handler_kind_by_method!(options, Method::OPTIONS);
handler_kind_by_method!(connect, Method::CONNECT);
handler_kind_by_method!(trace, Method::TRACE);

pub fn method<M, H, Mark>(method: M, handler: H) -> HandlerKind
where
	M: AsRef<str>,
	H: IntoHandler<Mark, IncomingBody>,
	H::Handler: Handler + Send + Sync + 'static,
	<H::Handler as Handler>::Response: IntoResponse,
{
	let ready_handler = ResponseFutureBoxer::wrap(IntoResponseAdapter::wrap(handler.into_handler()));

	let method = Method::from_str(method.as_ref())
		.expect("HTTP method should be a valid token [RFC 9110, 5.6.2 Tokens]");

	HandlerKind(Inner::Method(method, ready_handler.into_arc_handler()))
}

pub fn all_method<H, M>(handler: H) -> HandlerKind
where
	H: IntoHandler<M, IncomingBody>,
	H::Handler: Handler + Send + Sync + 'static,
	<H::Handler as Handler>::Response: IntoResponse,
{
	let ready_handler = ResponseFutureBoxer::wrap(IntoResponseAdapter::wrap(handler.into_handler()));

	HandlerKind(Inner::AllMethods(ready_handler.into_arc_handler()))
}

pub fn misdirected_request<H, M>(handler: H) -> HandlerKind
where
	H: IntoHandler<M, IncomingBody>,
	H::Handler: Handler + Send + Sync + 'static,
	<H::Handler as Handler>::Response: IntoResponse,
{
	let ready_handler = ResponseFutureBoxer::wrap(IntoResponseAdapter::wrap(handler.into_handler()));

	HandlerKind(Inner::MisdirectedRequest(ready_handler.into_arc_handler()))
}

// --------------------------------------------------------------------------------
