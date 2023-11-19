use std::{
	fmt::Debug,
	future::{ready, Ready},
};

use http::{HeaderName, HeaderValue, Method, StatusCode};

use crate::{
	body::IncomingBody,
	middleware::Layer,
	request::Request,
	response::{IntoResponse, Response},
	routing::UnusedRequest,
	utils::{BoxedFuture, Uncloneable},
};

use super::{wrap_arc_handler, AdaptiveHandler, ArcHandler, Handler};

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

#[derive(Clone)]
pub(crate) struct MethodHandlers {
	method_handlers: Vec<(Method, ArcHandler)>,
	unsupported_method_handler: Option<ArcHandler>,
}

impl MethodHandlers {
	pub(crate) fn new() -> MethodHandlers {
		MethodHandlers {
			method_handlers: Vec::new(),
			unsupported_method_handler: None,
		}
	}

	// ----------

	#[inline(always)]
	pub(crate) fn count(&self) -> usize {
		self.method_handlers.len()
	}

	#[inline(always)]
	pub(crate) fn is_empty(&self) -> bool {
		self.method_handlers.is_empty()
	}

	#[inline(always)]
	pub(crate) fn has_some_effect(&self) -> bool {
		!self.method_handlers.is_empty() || self.unsupported_method_handler.is_some()
	}

	#[inline]
	pub(crate) fn set_handler(&mut self, method: Method, handler: ArcHandler) {
		if self.method_handlers.iter().any(|(m, _)| m == method) {
			panic!("{} handler already exists", method)
		}

		self.method_handlers.push((method, handler));
	}

	#[inline]
	pub(crate) fn wrap_handler<L, LayeredB>(&mut self, method: Method, layer: L)
	where
		L: Layer<AdaptiveHandler<LayeredB>, LayeredB>,
		L::Handler: Handler<IncomingBody> + Send + Sync + 'static,
		<L::Handler as Handler<IncomingBody>>::Response: IntoResponse,
	{
		let Some(position) = self.method_handlers.iter().position(|(m, _)| m == method) else {
			panic!("'{}' handler doesn't exists", method)
		};

		let (method, boxed_handler) = std::mem::take(&mut self.method_handlers[position]);
		let boxed_handler = wrap_arc_handler(boxed_handler, layer);

		self.method_handlers[position] = (method, boxed_handler);
	}

	#[inline]
	pub(crate) fn allowed_methods(&self) -> AllowedMethods {
		let mut list = String::new();
		self
			.method_handlers
			.iter()
			.for_each(|(method, _)| list.push_str(method.as_str()));

		AllowedMethods(list)
	}

	#[inline]
	pub(crate) fn has_layered_unsupported_method_handler(&self) -> bool {
		self.unsupported_method_handler.is_some()
	}

	// ----------

	#[inline]
	pub(crate) fn handle(&self, mut request: Request) -> BoxedFuture<Response> {
		let method = request.method().clone();
		let some_handler = self
			.method_handlers
			.iter()
			.find(|(m, _)| m == method)
			.map(|(_, h)| h.clone());

		match some_handler {
			Some(handler) => handler.handle(request),
			None => {
				let allowed_methods = self.allowed_methods();
				request.extensions_mut().insert(Uncloneable::from(allowed_methods));

				match self.unsupported_method_handler.as_ref() {
					Some(unsupported_method_handler) => unsupported_method_handler.handle(request),
					None => Box::pin(handle_unsupported_method(request)),
				}
			}
		}
	}
}

impl Debug for MethodHandlers {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(
			f,
			"MethodHandlers {{ method_handlers_count: {}, unsupported_method_handler_exists: {} }}",
			self.method_handlers.len(),
			self.unsupported_method_handler.is_some(),
		)
	}
}

// --------------------------------------------------

pub(crate) struct AllowedMethods(String);

#[inline(always)]
async fn handle_unsupported_method(mut request: Request<IncomingBody>) -> Response {
	let allowed_methods = request
		.extensions_mut()
		.remove::<Uncloneable<AllowedMethods>>()
		.expect("Uncloneable<AllowedMethods> should be inserted by MethodHandlers instance")
		.into_inner()
		.expect("AllowedMethods should always exist in Uncloneable");

	let allowed_methods_header_value =
		HeaderValue::from_str(&allowed_methods.0).expect("method name should be a valid header value");

	let mut response = Response::default();
	*response.status_mut() = StatusCode::METHOD_NOT_ALLOWED;
	response.headers_mut().append(
		HeaderName::from_static("Allow"),
		allowed_methods_header_value,
	);

	response
}

#[inline]
pub(crate) fn misdirected_request_handler(request: Request<IncomingBody>) -> Ready<Response> {
	let mut response = Response::default();
	*response.status_mut() = StatusCode::NOT_FOUND;
	response
		.extensions_mut()
		.insert(Uncloneable::from(UnusedRequest::from(request)));

	ready(response)
}

// --------------------------------------------------------------------------------
