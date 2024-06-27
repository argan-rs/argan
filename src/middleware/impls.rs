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
	/// Creates a new `ErrorHandlerLayer` from an `ErrorHandler`.
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
// RedirectionLayer

/// A layer that applies a redirector middleware to a [`Handler`].
///
/// A middleware doesn't call the handler. That's why it replaces it instead of wrapping it.
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
	/// A permanent redirection to the provided URI.
	pub fn for_permanent_redirection_to(uri: U) -> Self {
		Self {
			status_code: StatusCode::PERMANENT_REDIRECT,
			prefix: false,
			uri,
		}
	}

	/// A permanent redirection to a new URI that's formed by joining
	/// the provided URI and the request's path.
	pub fn for_permanent_redirection_to_prefix(uri: U) -> Self {
		Self {
			status_code: StatusCode::PERMANENT_REDIRECT,
			prefix: true,
			uri,
		}
	}

	/// A temporary redirection to the provided URI.
	pub fn for_temporary_redirection_to(uri: U) -> Self {
		Self {
			status_code: StatusCode::TEMPORARY_REDIRECT,
			prefix: false,
			uri,
		}
	}

	/// A temporary redirection to a new URI that's formed by joining
	/// the provided URI and the request's path.
	pub fn for_temporary_redirection_to_prefix(uri: U) -> Self {
		Self {
			status_code: StatusCode::TEMPORARY_REDIRECT,
			prefix: true,
			uri,
		}
	}

	/// A redirection to see the provided URI.
	pub fn for_redirection_to_see(uri: U) -> Self {
		Self {
			status_code: StatusCode::SEE_OTHER,
			prefix: false,
			uri,
		}
	}

	/// A redirection to see a new URI that's formed by joining
	/// the provided URI and the request's path.
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
	type Handler = RedirectorHandler;

	fn wrap(&self, _handler: H) -> Self::Handler {
		RedirectorHandler::new(self.prefix, self.uri.clone())
	}
}

// -------------------------

mod private {
	use std::future::ready;

	use crate::response::Redirect;

	use super::*;

	// -------------------------
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

	// -------------------------
	// RedirectorHandler

	#[derive(Clone)]
	pub struct RedirectorHandler {
		prefix: bool,
		uri: Box<str>,
	}

	impl RedirectorHandler {
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

	impl<B, Ext: Clone> Handler<B, Ext> for RedirectorHandler {
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
}

use http::StatusCode;
pub(crate) use private::RedirectorHandler;
pub(crate) use private::ResponseResultHandler;

// --------------------------------------------------------------------------------
