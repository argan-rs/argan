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
/// resource.wrap(RequestHandler.with(ErrorHandlerLayer::new(error_handler)));
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

mod private {
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
}

pub(crate) use private::ResponseResultHandler;

// --------------------------------------------------------------------------------
