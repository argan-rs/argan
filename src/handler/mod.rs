pub use hyper::service::Service;

use crate::{
	body::Incoming,
	request::Request,
	response::Response,
	utils::{BoxedError, BoxedFuture},
};

// --------------------------------------------------
pub mod implementation;

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

pub trait Handler<RqB = Incoming>
where
	Self:
		Service<Request<RqB>, Response = Response, Error = BoxedError> + Clone + Send + Sync + 'static,
{
}

impl<S, RqB> Handler<RqB> for S where
	S: Service<Request<RqB>, Response = Response, Error = BoxedError> + Clone + Send + Sync + 'static
{
}

// -------------------------

pub trait IntoHandler<H, RqB = Incoming>
where
	H: Handler<RqB>,
{
	fn into_handler(self) -> H;
}

impl<H, RqB> IntoHandler<H, RqB> for H
where
	H: Handler<RqB>,
{
	#[inline]
	fn into_handler(self) -> H {
		self
	}
}

// --------------------------------------------------------------------------------

pub(crate) type BoxedHandler<RqB> = Box<
	dyn CloneableHandler<
		RqB,
		Response = Response,
		Error = BoxedError,
		Future = BoxedFuture<Result<Response, BoxedError>>,
	>,
>;

pub(crate) trait CloneableHandler<RqB>:
	Service<Request<RqB>> + CloneBoxedHandler<RqB> + Send + Sync
{
}

impl<H, RqB> CloneableHandler<RqB> for H where
	H: Handler<
		RqB,
		Response = Response,
		Error = BoxedError,
		Future = BoxedFuture<Result<Response, BoxedError>>,
	>
{
}

pub(crate) trait CloneBoxedHandler<RqB> {
	fn clone_boxed(&self) -> BoxedHandler<RqB>;
}

impl<H, RqB> CloneBoxedHandler<RqB> for H
where
	H: Handler<
		RqB,
		Response = Response,
		Error = BoxedError,
		Future = BoxedFuture<Result<Response, BoxedError>>,
	>,
{
	fn clone_boxed(&self) -> BoxedHandler<RqB> {
		Box::new(self.clone())
	}
}

impl<H, RqB> From<H> for BoxedHandler<RqB>
where
	H: Handler<
		RqB,
		Response = Response,
		Error = BoxedError,
		Future = BoxedFuture<Result<Response, BoxedError>>,
	>,
{
	fn from(value: H) -> Self {
		Box::new(value)
	}
}
