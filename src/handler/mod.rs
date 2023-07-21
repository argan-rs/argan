pub use hyper::service::Service;

use crate::{
	body::Incoming,
	request::Request,
	response::Response,
	utils::{BoxedError, BoxedFuture},
};

// --------------------------------------------------

pub mod impls;
pub(crate) mod request_handler;

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

// --------------------------------------------------------------------------------

pub(crate) struct HandlerService<RqB>(BoxedHandler<RqB>);

impl<RqB> Clone for HandlerService<RqB> {
	fn clone(&self) -> Self {
		HandlerService(self.0.clone_boxed())
	}
}

impl<RqB> Service<Request<RqB>> for HandlerService<RqB> {
	type Response = Response;
	type Error = BoxedError;
	type Future = BoxedFuture<Result<Response, BoxedError>>;

	#[inline]
	fn call(&mut self, req: Request<RqB>) -> Self::Future {
		self.0.call(req)
	}
}

impl<RqB> HandlerService<RqB> {
	fn wrap_with<L>(self, wrapper: L) -> HandlerService<RqB>
	where
		L: Layer<HandlerService<RqB>>,
		L::Service: Handler<
			RqB,
			Response = Response,
			Error = BoxedError,
			Future = BoxedFuture<Result<Response, BoxedError>>,
		>,
	{
		HandlerService(BoxedHandler::from(wrapper.layer(self)))
	}
}

impl<RqB> HandlerService<RqB> {
	fn from_handler<IH, H>(value: IH) -> Self
	where
		IH: IntoHandler<H, RqB>,
		H: Handler<
			RqB,
			Response = Response,
			Error = BoxedError,
			Future = BoxedFuture<Result<Response, BoxedError>>,
		>,
	{
		HandlerService(BoxedHandler::from(value.into_handler()))
	}
}

// --------------------------------------------------------------------------------

pub trait Layer<S> {
	type Service;

	fn layer(&self, inner: S) -> Self::Service;
}

// --------------------------------------------------
// --------------------------------------------------

#[cfg(test)]
mod test {
	use std::marker::PhantomData;

	use super::*;

	// --------------------------------------------------
	// --------------------------------------------------

	async fn handler(req: Request) -> Result<Response, BoxedError> {
		unimplemented!()
	}

	struct Wrapper<H, RqB> {
		inner: H,
		_mark: PhantomData<RqB>,
	}

	impl<H, RqB> Clone for Wrapper<H, RqB>
	where
		H: Clone,
	{
		fn clone(&self) -> Self {
			Wrapper {
				inner: self.inner.clone(),
				_mark: PhantomData,
			}
		}
	}

	impl<H, RqB> Service<Request<RqB>> for Wrapper<H, RqB>
	where
		H: Handler<
			RqB,
			Response = Response,
			Error = BoxedError,
			Future = BoxedFuture<Result<Response, BoxedError>>,
		>,
	{
		type Response = H::Response;
		type Error = H::Error;
		type Future = H::Future;

		fn call(&mut self, req: Request<RqB>) -> Self::Future {
			self.inner.call(req)
		}
	}

	struct WrapperLayer<RqB> {
		_mark: PhantomData<RqB>,
	}

	impl<H, RqB> Layer<H> for WrapperLayer<RqB> {
		type Service = Wrapper<H, RqB>;

		fn layer(&self, inner: H) -> Self::Service {
			Wrapper {
				inner,
				_mark: PhantomData,
			}
		}
	}

	fn is_handler<H, RqB>(_: H)
	where
		H: CloneableHandler<
			RqB,
			Response = Response,
			Error = BoxedError,
			Future = BoxedFuture<Result<Response, BoxedError>>,
		>,
	{
	}

	fn is_into_handler<IH, H, RqB>(_: IH)
	where
		IH: IntoHandler<H, RqB>,
		H: Handler<RqB, Response = Response, Error = BoxedError>,
	{
	}

	fn test() {
		let hs = HandlerService(BoxedHandler::from(handler.into_handler()));
		let boxed = hs.clone_boxed();
		is_handler(hs);

		let mut hs = HandlerService(BoxedHandler::from(handler.into_handler()));
		hs = hs.wrap_with(WrapperLayer { _mark: PhantomData });
		is_handler(hs);

		let hs = HandlerService::from_handler(handler);
		is_handler(hs);
	}
}
