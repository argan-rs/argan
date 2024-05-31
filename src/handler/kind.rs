//! Request handler kinds.

// ----------

use std::str::FromStr;

use argan_core::body::Body;
use http::Method;

use crate::{
	common::marker::Sealed,
	http::{CustomMethod, WildcardMethod},
	request::MistargetedRequest,
};

use super::{BoxableHandler, BoxedHandler, FinalHandler, IntoHandler};

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

/// A trait that's implemented by the [`Method`](crate::http::Method) type to pass
/// the given `handler` as a *method* handler.
pub trait HandlerSetter: Sealed {
	fn to<H, Mark>(self, handler: H) -> HandlerKind
	where
		H: IntoHandler<Mark, Body>,
		H::Handler: BoxableHandler + Clone + Send + Sync + 'static;
}

impl HandlerSetter for Method {
	fn to<H, Mark>(self, handler: H) -> HandlerKind
	where
		H: IntoHandler<Mark, Body>,
		H::Handler: BoxableHandler + Clone + Send + Sync + 'static,
	{
		let final_handler = handler.into_handler();

		HandlerKind::Method(self, final_handler.into_boxed_handler())
	}
}

// --------------------------------------------------
// CustomMethod

impl<M: AsRef<str>> CustomMethod<M> {
	/// Passes the given `handler` as a *custom HTTP method* handler.
	pub fn to<H, Mark>(self, handler: H) -> HandlerKind
	where
		H: IntoHandler<Mark, Body>,
		H::Handler: BoxableHandler + Clone + Send + Sync + 'static,
	{
		let method = Method::from_str(self.0.as_ref())
			.expect("HTTP method should be a valid token [RFC 9110, 5.6.2 Tokens]");

		let final_handler = handler.into_handler();

		HandlerKind::Method(method, final_handler.into_boxed_handler())
	}
}

// --------------------------------------------------
// WildcardMethod

impl WildcardMethod {
	/// A *wildcard method* handler is called when there is no dedicated handler
	/// for the request's method.
	///
	/// When there is a *method* handler or more, the default *wildcard method* handler responds
	/// with an `"Allow"` header listing the supported methods. To set the custom *wildcard method*
	/// handler, pass `Some(handler)`. To turn this functionality off, pass `None`.
	pub fn to<H, Mark>(self, some_handler: Option<H>) -> HandlerKind
	where
		H: IntoHandler<Mark, Body>,
		H::Handler: BoxableHandler + Clone + Send + Sync + 'static,
	{
		let some_final_handler = some_handler.map(|handler| {
			let final_handler = handler.into_handler();

			final_handler.into_boxed_handler()
		});

		HandlerKind::WildcardMethod(some_final_handler)
	}
}

// --------------------------------------------------
// MistargetedRequest

impl MistargetedRequest {
	/// Passes the given `handler` as a *mistargeted request* handler.
	///
	/// A *mistargeted request* handler is called when there is no resource among
	/// subresources matching the request’s path.
	pub fn to<H, Mark>(self, handler: H) -> HandlerKind
	where
		H: IntoHandler<Mark, Body>,
		H::Handler: BoxableHandler + Clone + Send + Sync + 'static,
	{
		let final_handler = handler.into_handler();

		HandlerKind::MistargetedRequest(final_handler.into_boxed_handler())
	}
}

// --------------------------------------------------------------------------------
