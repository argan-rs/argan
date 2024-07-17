use crate::{
	handler::{Args, ErrorHandler},
	request::RequestContext,
};

use super::*;

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

// --------------------------------------------------
// ErrorHandlerLayer

/// A layer that applies an error handler middleware to a [`Handler`].
///
/// ```
/// use argan::{
///   Resource,
///   response::{Response, BoxedErrorResponse},
///   middleware::{RequestHandler, ErrorHandlerLayer},
/// };
///
/// async fn error_handler(error: BoxedErrorResponse) -> Result<Response, BoxedErrorResponse> {
///   eprintln!("Error: {}", error);
///
///   Ok(error.into_response())
/// }
///
/// let mut resource = Resource::new("/resource");
/// resource.wrap(RequestHandler.component_in(ErrorHandlerLayer::new(error_handler)));
/// ```
#[derive(Clone)]
pub struct ErrorHandlerLayer<ErrH>(ErrH);

impl<ErrH> ErrorHandlerLayer<ErrH>
where
	ErrH: ErrorHandler + Clone,
{
	/// Creates a new `ErrorHandlerLayer` from an `ErrorHandler` implementor.
	pub fn new(error_handler: ErrH) -> Self {
		Self(error_handler)
	}
}

impl<H, ErrH> Layer<H> for ErrorHandlerLayer<ErrH>
where
	ErrH: ErrorHandler + Clone,
{
	type Handler = ResponseResultHandler<H, ErrH>;

	fn wrap(&self, handler: H) -> Self::Handler {
		ResponseResultHandler::new(handler, self.0.clone())
	}
}

// --------------------------------------------------
// RequestExtensionsModifierLayer

/// A layer that applies a request extensions modifier middleware to a [`Handler`].
///
/// ```
/// use argan::{
///   Router,
///   Resource,
///   request::RequestHead,
///   response::{Response, BoxedErrorResponse},
///   http::Method,
///   handler::HandlerSetter,
///   middleware::{RequestReceiver, RequestPasser, RequestExtensionsModifierLayer},
/// };
///
/// #[derive(Clone)]
/// struct State1 {
///   // ...
/// }
///
/// #[derive(Clone)]
/// struct State2 {
///   // ...
/// }
///
/// async fn handler(request_head: RequestHead) {
///   let extensions = request_head.extensions_ref();
///
///   let state1 = extensions
///     .get::<State1>()
///     .expect("`Router` should have inserted the `State1`");
///
///   let state2 = extensions
///     .get::<State2>()
///     .expect("the `State2` should have been inserted up in the resource tree");
///
///   // ...
/// }
///
/// // ...
///
/// let state1 = State1 { /* ... */ };
/// let state2 = State2 { /* ... */ };
///
/// let mut router = Router::new();
/// router.wrap(RequestPasser.component_in(
///   RequestExtensionsModifierLayer::new(move |extensions| {
///   let state1_clone = state1.clone();
///   extensions.insert(state1_clone);
/// })));
///
/// let mut resource0 = Resource::new("/resource0");
/// resource0.wrap(RequestReceiver.component_in(
///   RequestExtensionsModifierLayer::new(move |extensions| {
///   let state2_clone = state2.clone();
///   extensions.insert(state2_clone);
/// })));
///
/// resource0
///   .subresource_mut("/resource1/resource2")
///   .set_handler_for(Method::GET.to(handler));
///
/// router.add_resource(resource0);
/// ```
#[derive(Clone)]
pub struct RequestExtensionsModifierLayer(BoxedExtensionsModifier);

impl RequestExtensionsModifierLayer {
	/// Creates a new `RequestExtensionsModifierLayer` from a function or a closure.
	pub fn new<Func>(modifier: Func) -> Self
	where
		Func: Fn(&mut Extensions) + Clone + Send + Sync + 'static,
	{
		Self(BoxedExtensionsModifier::new(modifier))
	}
}

impl<H> Layer<H> for RequestExtensionsModifierLayer {
	type Handler = RequestExtensionsModifier<H>;

	fn wrap(&self, handler: H) -> Self::Handler {
		RequestExtensionsModifier::new(handler, self.0.clone())
	}
}

// -------------------------

trait ExtensionsModifier: Fn(&mut Extensions) {
	fn boxed_clone(&self) -> BoxedExtensionsModifier;
}

impl<Func> ExtensionsModifier for Func
where
	Func: Fn(&mut Extensions) + Clone + Send + Sync + 'static,
{
	fn boxed_clone(&self) -> BoxedExtensionsModifier {
		BoxedExtensionsModifier::new(self.clone())
	}
}

// -------------------------

struct BoxedExtensionsModifier(Box<dyn ExtensionsModifier + Send + Sync + 'static>);

impl BoxedExtensionsModifier {
	pub(crate) fn new<Func>(modifier: Func) -> Self
	where
		Func: Fn(&mut Extensions) + Clone + Send + Sync + 'static,
	{
		Self(Box::new(modifier))
	}
}

impl Clone for BoxedExtensionsModifier {
	fn clone(&self) -> Self {
		self.0.boxed_clone()
	}
}

// --------------------------------------------------
// RedirectionLayer

/// A layer that applies a redirector middleware to a [`Handler`].
///
/// `RedirectionLayer` doesn't call the handler. That's why it replaces it instead of wrapping it.
///
/// ```
/// use argan::{
///   Router,
///   Resource,
///   middleware::{RequestReceiver, RedirectionLayer},
/// };
///
/// let mut router = Router::new();
/// let mut root = router.resource_mut("http://www.example.com/");
///
/// // ...
///
/// router
///   .resource_mut("http://example.com/")
///   .wrap(RequestReceiver.component_in(
///     RedirectionLayer::for_permanent_redirection_to_prefix("http://www.example.com"),
///   ));
/// ```
#[derive(Clone)]
pub struct RedirectionLayer<U: AsRef<str>> {
	status_code: StatusCode,
	prefix: bool,
	uri: U,
}

impl<U: AsRef<str>> RedirectionLayer<U> {
	/// A permanent redirection to the provided URI with a status code of `308`.
	pub fn for_permanent_redirection_to(uri: U) -> Self {
		Self {
			status_code: StatusCode::PERMANENT_REDIRECT,
			prefix: false,
			uri,
		}
	}

	/// A permanent redirection to a new URI that's formed by joining
	/// the provided URI and the request's path.
	///
	/// The status code of the response is `308`.
	pub fn for_permanent_redirection_to_prefix(uri: U) -> Self {
		Self {
			status_code: StatusCode::PERMANENT_REDIRECT,
			prefix: true,
			uri,
		}
	}

	/// A temporary redirection to the provided URI with a status code of `307`.
	pub fn for_temporary_redirection_to(uri: U) -> Self {
		Self {
			status_code: StatusCode::TEMPORARY_REDIRECT,
			prefix: false,
			uri,
		}
	}

	/// A temporary redirection to a new URI that's formed by joining
	/// the provided URI and the request's path.
	///
	/// The status code of the response is `307`.
	pub fn for_temporary_redirection_to_prefix(uri: U) -> Self {
		Self {
			status_code: StatusCode::TEMPORARY_REDIRECT,
			prefix: true,
			uri,
		}
	}

	/// A redirection to see the provided URI with a status code of `303`.
	pub fn for_redirection_to_see(uri: U) -> Self {
		Self {
			status_code: StatusCode::SEE_OTHER,
			prefix: false,
			uri,
		}
	}

	/// A redirection to see a new URI that's formed by joining
	/// the provided URI and the request's path.
	///
	/// The status code of the response is `303`.
	pub fn for_redirection_to_see_prefix(uri: U) -> Self {
		Self {
			status_code: StatusCode::SEE_OTHER,
			prefix: true,
			uri,
		}
	}
}

impl<H, U> Layer<H> for RedirectionLayer<U>
where
	U: AsRef<str> + Clone,
{
	type Handler = Redirector;

	fn wrap(&self, _handler: H) -> Self::Handler {
		Redirector::new(self.prefix, self.uri.clone())
	}
}

// --------------------------------------------------------------------------------

mod private {
	use std::future::ready;

	use crate::response::Redirect;

	use super::*;

	// --------------------------------------------------
	// ResponseResultHandler

	#[derive(Clone)]
	pub struct ResponseResultHandler<H, ErrH> {
		inner: H,
		error_handler: ErrH,
	}

	impl<H, ErrH> ResponseResultHandler<H, ErrH> {
		pub(crate) fn new(inner: H, error_handler: ErrH) -> Self {
			Self {
				inner,
				error_handler,
			}
		}
	}

	impl<H, B, Ext, ErrH> Handler<B, Ext> for ResponseResultHandler<H, ErrH>
	where
		H: Handler<
			B,
			Ext,
			Response = Response,
			Error = BoxedErrorResponse,
			Future = BoxedFuture<Result<Response, BoxedErrorResponse>>,
		>,
		Ext: Clone,
		ErrH: ErrorHandler + Clone + Send + 'static,
	{
		type Response = Response;
		type Error = BoxedErrorResponse;
		type Future = BoxedFuture<Result<Self::Response, Self::Error>>;

		#[inline]
		fn handle(&self, request_context: RequestContext<B>, args: Args<'_, Ext>) -> Self::Future {
			let future = self.inner.handle(request_context, args);
			let mut error_handler_clone = self.error_handler.clone();

			Box::pin(async move {
				match future.await {
					Ok(response) => Ok(response.into_response()),
					Err(error) => error_handler_clone.handle_error(error).await,
				}
			})
		}
	}

	// --------------------------------------------------
	// RequestExtensionsModifier

	#[derive(Clone)]
	pub struct RequestExtensionsModifier<H> {
		inner_handler: H,
		boxed_modifier: BoxedExtensionsModifier,
	}

	impl<H> RequestExtensionsModifier<H> {
		pub(super) fn new(handler: H, boxed_modifier: BoxedExtensionsModifier) -> Self {
			Self {
				inner_handler: handler,
				boxed_modifier,
			}
		}
	}

	impl<H, B> Handler<B> for RequestExtensionsModifier<H>
	where
		H: Handler<B>,
	{
		type Response = H::Response;
		type Error = H::Error;
		type Future = H::Future;

		#[inline(always)]
		fn handle(&self, mut request_context: RequestContext<B>, args: Args<'_, ()>) -> Self::Future {
			self.boxed_modifier.0(request_context.request_mut().extensions_mut());

			self.inner_handler.handle(request_context, args)
		}
	}

	// --------------------------------------------------
	// RedirectorHandler

	#[derive(Clone)]
	pub struct Redirector {
		prefix: bool,
		uri: Box<str>,
	}

	impl Redirector {
		pub(crate) fn new<U: AsRef<str>>(prefix: bool, uri: U) -> Self {
			let uri = uri.as_ref();

			let uri = if prefix {
				// The request's path always starts with a slash '/'.
				// So, we don't need a trailing slash in the URI prefix.
				uri.strip_suffix('/').unwrap_or(uri)
			} else {
				uri
			};

			Self {
				prefix,
				uri: uri.into(),
			}
		}
	}

	impl<B, Ext: Clone> Handler<B, Ext> for Redirector {
		type Response = Response;
		type Error = BoxedErrorResponse;
		type Future = BoxedFuture<Result<Self::Response, Self::Error>>;

		#[inline]
		fn handle(&self, request_context: RequestContext<B>, _args: Args<'_, Ext>) -> Self::Future {
			let redirect = if self.prefix {
				let uri = format!("{}{}", self.uri.as_ref(), request_context.uri_ref().path());

				Redirect::permanently_to(uri)
			} else {
				Redirect::permanently_to(self.uri.as_ref())
			};

			Box::pin(ready(Ok(redirect.into_response())))
		}
	}

	// --------------------------------------------------
	// LayerFn

	#[derive(Clone)]
	pub struct LayerFn<Func>(pub(crate) Func);

	impl<Func, InH, OutH> Layer<InH> for LayerFn<Func>
	where
		Func: Fn(InH) -> OutH,
	{
		type Handler = OutH;

		fn wrap(&self, handler: InH) -> Self::Handler {
			self.0(handler)
		}
	}
}

use http::{Extensions, StatusCode};
pub(crate) use private::LayerFn;
pub(crate) use private::Redirector;
pub(crate) use private::RequestExtensionsModifier;
pub(crate) use private::ResponseResultHandler;

// --------------------------------------------------------------------------------
