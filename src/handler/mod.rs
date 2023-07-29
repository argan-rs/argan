use std::fmt::Debug;

pub use hyper::service::Service;

use crate::{
	body::Body,
	body::IncomingBody,
	request::Request,
	response::{IntoResponse, Response},
	utils::{BoxedError, BoxedFuture},
};

// --------------------------------------------------

pub mod impls;
pub(crate) mod request_handler;

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

pub trait Handler
where
	Self: Service<Request<IncomingBody>> + Send + Sync + 'static,
	Self::Response: IntoResponse,
	Self::Error: Into<BoxedError>,
{
}

impl<S> Handler for S
where
	S: Service<Request<IncomingBody>> + Send + Sync + 'static,
	Self::Response: IntoResponse,
	Self::Error: Into<BoxedError>,
{
}

// -------------------------

pub trait IntoHandler<H, S = ()>
where
	Self: Sized,
	H: Handler,
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

impl<H, S> IntoHandler<H, S> for H
where
	H: Handler,
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

impl<H, S> Service<Request<IncomingBody>> for StatefulHandler<H, S>
where
	H: Handler,
	H::Response: IntoResponse,
	H::Error: Into<BoxedError>,
	S: Clone + Send + Sync + 'static,
{
	type Response = H::Response;
	type Error = H::Error;
	type Future = H::Future;

	#[inline]
	fn call(&self, mut req: Request<IncomingBody>) -> Self::Future {
		if let Some(_previous_state_with_the_same_type) = req
			.extensions_mut()
			.insert(HandlerState(self.state.clone()))
		{
			panic!("state with the same type exists")
		}

		self.inner.call(req)
	}
}

// --------------------------------------------------

pub struct HandlerState<S>(S);

// --------------------------------------------------

pub(crate) struct AdapterHandler<H>(H);

impl<H, B> Service<Request<B>> for AdapterHandler<H>
where
	H: Handler,
	H::Response: IntoResponse,
	H::Error: Into<BoxedError>,
	B: Body + Send + Sync + 'static,
	B::Data: Debug,
	B::Error: Into<BoxedError>,
{
	type Response = Response;
	type Error = BoxedError;
	type Future = BoxedFuture<Result<Self::Response, Self::Error>>;

	fn call(&self, req: Request<B>) -> Self::Future {
		let (parts, body) = req.into_parts();
		let body = IncomingBody::new(body);
		let req = Request::from_parts(parts, body);

		let future_result = self.0.call(req);

		Box::pin(async move {
			match future_result.await {
				Ok(res) => Ok(res.into_response()),
				Err(err) => Err(err.into()),
			}
		})
	}
}

impl<H> AdapterHandler<H> {
	#[inline]
	fn new(handler: H) -> Self {
		Self(handler)
	}
}

// --------------------------------------------------

pub(crate) type BoxedHandler = Box<
	dyn Handler<
		Response = Response,
		Error = BoxedError,
		Future = BoxedFuture<Result<Response, BoxedError>>,
	>,
>;

impl<H> From<H> for BoxedHandler
where
	H: Handler<
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
