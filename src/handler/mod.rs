use std::marker::PhantomData;

pub use hyper::service::Service;

use crate::{
	body::Incoming,
	request::Request,
	response::{IntoResponse, Response},
	utils::{BoxedError, BoxedFuture},
};

// --------------------------------------------------

pub mod impls;
pub(crate) mod request_handler;

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

pub trait Handler<B = Incoming>
where
	Self: Service<Request<B>> + Send + Sync + 'static,
	Self::Response: IntoResponse,
	Self::Error: Into<BoxedError>,
{
}

impl<S, B> Handler<B> for S
where
	S: Service<Request<B>> + Send + Sync + 'static,
	Self::Response: IntoResponse,
	Self::Error: Into<BoxedError>,
{
}

// -------------------------

pub trait IntoHandler<H, B = Incoming, S = ()>
where
	Self: Sized,
	H: Handler<B>,
	H::Response: IntoResponse,
	H::Error: Into<BoxedError>,
{
	fn into_handler(self) -> H;

	#[inline]
	fn with_state(self, state: S) -> StatefulHandler<H, S> {
		StatefulHandler {
			inner: self.into_handler(),
			state,
		}
	}
}

impl<H, B, S> IntoHandler<H, B, S> for H
where
	H: Handler<B>,
	H::Response: IntoResponse,
	H::Error: Into<BoxedError>,
{
	#[inline]
	fn into_handler(self) -> H {
		self
	}
}

// --------------------------------------------------

pub struct StatefulHandler<H, S> {
	inner: H,
	state: S,
}

impl<H, S, B> Service<Request<B>> for StatefulHandler<H, S>
where
	H: Handler<B>,
	H::Response: IntoResponse,
	H::Error: Into<BoxedError>,
	S: Clone + Send + Sync + 'static,
{
	type Response = H::Response;
	type Error = H::Error;
	type Future = H::Future;

	#[inline]
	fn call(&self, mut req: Request<B>) -> Self::Future {
		if let Some(_previous_state_with_the_same_type) = req
			.extensions_mut()
			.insert(HandlerState(self.state.clone()))
		{
			// TODO: Improve the error message by implementing Debug for the HandlerState.
			panic!("multiple insertions of a state with the same type")
		}

		self.inner.call(req)
	}
}

// --------------------------------------------------

// TODO: Is this a good place?
pub struct HandlerState<S>(S);

// --------------------------------------------------

pub(crate) struct HandlerService<H>(H);

impl<H, B> Service<Request<B>> for HandlerService<H>
where
	H: Handler<B>,
	B: 'static,
	H::Response: IntoResponse,
	H::Error: Into<BoxedError>,
{
	type Response = Response;
	type Error = BoxedError;
	type Future = BoxedFuture<Result<Self::Response, Self::Error>>;

	fn call(&self, req: Request<B>) -> Self::Future {
		let future_result = self.0.call(req);

		Box::pin(async move {
			match future_result.await {
				Ok(res) => Ok(res.into_response()),
				Err(err) => Err(err.into()),
			}
		})
	}
}

impl<H> HandlerService<H> {
	#[inline]
	fn new(handler: H) -> Self {
		Self(handler)
	}
}

// --------------------------------------------------

pub(crate) type BoxedHandler<B> = Box<
	dyn Handler<
		B,
		Response = Response,
		Error = BoxedError,
		Future = BoxedFuture<Result<Response, BoxedError>>,
	>,
>;

// pub(crate) type BoxedHandler<B> = Box<
// 	dyn CloneableHandler<
// 		B,
// 		Response = Response,
// 		Error = BoxedError,
// 		Future = BoxedFuture<Result<Response, BoxedError>>,
// 	>
// >;

// pub(crate) trait CloneableHandler<B>: Service<Request<B>> + /*CloneBoxedHandler<B> +*/ Send + Sync {}

// impl<H, B> CloneableHandler<B> for H
// where
// 	H: Handler<B, Response = Response, Error = BoxedError, Future = BoxedFuture<Result<Response, BoxedError>>>,
// {}

// pub(crate) trait CloneBoxedHandler<RqB> {
// 	fn clone_boxed(&self) -> BoxedHandler<RqB>;
// }

// impl<H, B> CloneBoxedHandler<B> for H
// where
// 	H: Handler<B, Response = Response, Error = BoxedError, Future = BoxedFuture<Result<Response, BoxedError>>>,
// {
// 	#[inline]
// 	fn clone_boxed(&self) -> BoxedHandler<B> {
// 		Box::new(self.clone())
// 	}
// }

impl<H, B> From<H> for BoxedHandler<B>
where
	H: Handler<
		B,
		Response = Response,
		Error = BoxedError,
		Future = BoxedFuture<Result<Response, BoxedError>>,
	>,
{
	#[inline]
	fn from(handler: H) -> Self {
		Box::new(handler)
	}
}

// --------------------------------------------------------------------------------

pub trait Layer<S> {
	type Service;

	fn layer(&self, inner: S) -> Self::Service;
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

	fn call(&self, req: Request<RqB>) -> Self::Future {
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
