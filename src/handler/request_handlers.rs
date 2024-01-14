use std::{
	fmt::Debug,
	future::{ready, Ready},
};

use http::{HeaderName, HeaderValue, Method, StatusCode};

use crate::{
	body::IncomingBody,
	middleware::{IntoResponseAdapter, Layer, ResponseFutureBoxer},
	request::Request,
	response::{IntoResponse, Response},
	routing::UnusedRequest,
	utils::{BoxedFuture, Uncloneable},
};

use super::{wrap_arc_handler, AdaptiveHandler, ArcHandler, Handler, IntoArcHandler, IntoHandler};

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

#[derive(Clone)]
pub(crate) struct MethodHandlers {
	list: Vec<(Method, ArcHandler)>,
	some_all_methods_handler: Option<ArcHandler>,
}

impl MethodHandlers {
	pub(crate) fn new() -> Self {
		MethodHandlers {
			list: Vec::new(),
			some_all_methods_handler: None,
		}
	}

	// ----------

	#[inline(always)]
	pub(crate) fn count(&self) -> usize {
		self.list.len()
	}

	#[inline(always)]
	pub(crate) fn is_empty(&self) -> bool {
		self.list.is_empty()
	}

	#[inline(always)]
	pub(crate) fn has_some_effect(&self) -> bool {
		!self.list.is_empty() || self.some_all_methods_handler.is_some()
	}

	#[inline]
	pub(crate) fn set_for(&mut self, method: Method, handler: ArcHandler) {
		if self.list.iter().any(|(m, _)| m == method) {
			panic!("{} handler already exists", method)
		}

		self.list.push((method, handler));
	}

	#[inline(always)]
	pub(crate) fn set_for_all_methods(&mut self, handler: ArcHandler) {
		self.some_all_methods_handler = Some(handler);
	}

	#[inline]
	pub(crate) fn wrap<L, LayeredB>(&mut self, method: Method, layer: L)
	where
		L: Layer<AdaptiveHandler<LayeredB>, LayeredB>,
		L::Handler: Handler<IncomingBody> + Send + Sync + 'static,
		<L::Handler as Handler<IncomingBody>>::Response: IntoResponse,
	{
		let Some(position) = self.list.iter().position(|(m, _)| m == method) else {
			panic!("'{}' handler doesn't exists", method)
		};

		let (method, boxed_handler) = std::mem::take(&mut self.list[position]);
		let boxed_handler = wrap_arc_handler(boxed_handler, layer);

		self.list[position] = (method, boxed_handler);
	}

	#[inline(always)]
	pub(crate) fn allowed_methods(&self) -> AllowedMethods {
		let mut list = String::new();
		self
			.list
			.iter()
			.for_each(|(method, _)| list.push_str(method.as_str()));

		AllowedMethods(list)
	}

	#[inline(always)]
	pub(crate) fn has_all_methods_handler(&self) -> bool {
		self.some_all_methods_handler.is_some()
	}

	// ----------

	#[inline]
	pub(crate) fn handle(&self, mut request: Request) -> BoxedFuture<Response> {
		let method = request.method().clone();
		let some_handler = self
			.list
			.iter()
			.find(|(m, _)| m == method)
			.map(|(_, h)| h.clone());

		match some_handler {
			Some(handler) => handler.handle(request),
			None => {
				let allowed_methods = self.allowed_methods();
				request
					.extensions_mut()
					.insert(Uncloneable::from(allowed_methods));

				match self.some_all_methods_handler.as_ref() {
					Some(unsupported_method_handler) => unsupported_method_handler.handle(request),
					None => Box::pin(handle_unimplemented_method(request)),
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
			self.list.len(),
			self.some_all_methods_handler.is_some(),
		)
	}
}

// --------------------------------------------------

pub(crate) struct AllowedMethods(String);

#[inline(always)]
async fn handle_unimplemented_method(mut request: Request<IncomingBody>) -> Response {
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
