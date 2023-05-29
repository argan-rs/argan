

// --------------------------------------------------

pub type Request = hyper::Request<hyper::Body>;
pub type Response = hyper::Response<hyper::Body>;
pub type BoxedError = tower::BoxError;
pub type BoxPinnedFuture = futures::future::BoxFuture<'static, Result<Response, BoxedError>>;
pub type BoxedService = Box<
	dyn tower::Service<Request, Response = Response, Error = BoxedError, Future = BoxPinnedFuture>
		+ Send
		+ Sync,
>;

