use std::sync::Arc;

use http::Method;
use tower_layer::Layer as TowerLayer;

use crate::{
	common::{BoxedError, BoxedFuture, IntoArray},
	handler::{AdaptiveHandler, BoxedHandler, Handler, HandlerService /* HandlerService */},
	response::{IntoResponse, Response},
};

// --------------------------------------------------

mod impls;
mod internal;

pub use impls::*;
pub(crate) use internal::*;

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

pub trait Layer<H> {
	type Handler;

	fn wrap(&self, handler: H) -> Self::Handler;
}

impl<TL, H> Layer<H> for TL
where
	TL: TowerLayer<HandlerService<H>>,
{
	type Handler = TL::Service;

	fn wrap(&self, handler: H) -> Self::Handler {
		self.layer(HandlerService::from(handler))
	}
}

// -------------------------

pub trait IntoLayer<Mark, H> {
	type Layer: Layer<H>;

	fn into_layer(self) -> Self::Layer;
}

impl<L, H> IntoLayer<(), H> for L
where
	L: Layer<H>,
{
	type Layer = L;

	fn into_layer(self) -> Self::Layer {
		self
	}
}

// --------------------------------------------------
// FinalLayer

trait FinalLayer
where
	Self: Layer<AdaptiveHandler, Handler = BoxedHandler>,
{
	fn into_boxed_layer(self) -> BoxedLayer;
	fn boxed_clone(&self) -> BoxedLayer;
}

impl<L> FinalLayer for L
where
	L: Layer<AdaptiveHandler, Handler = BoxedHandler> + Clone + 'static,
{
	fn into_boxed_layer(self) -> BoxedLayer {
		BoxedLayer(Box::new(self))
	}

	fn boxed_clone(&self) -> BoxedLayer {
		BoxedLayer(Box::new(self.clone()))
	}
}

// --------------------------------------------------
// BoxedLayer

pub(crate) struct BoxedLayer(Box<dyn FinalLayer>);

impl BoxedLayer {
	pub(crate) fn new<L>(layer: L) -> Self
	where
		L: Layer<AdaptiveHandler> + Clone + 'static,
		L::Handler:
			Handler<Response = Response, Future = BoxedFuture<Response>> + Clone + Send + Sync + 'static,
	{
		let adaptive_handler_wrapper = AdaptiveHandlerWrapper(layer);

		BoxedLayer(Box::new(adaptive_handler_wrapper))
	}
}

impl Layer<AdaptiveHandler> for BoxedLayer {
	type Handler = BoxedHandler;

	fn wrap(&self, handler: AdaptiveHandler) -> Self::Handler {
		self.0.as_ref().wrap(handler)
	}
}

impl Clone for BoxedLayer {
	fn clone(&self) -> Self {
		self.0.boxed_clone()
	}
}

// --------------------------------------------------
// AdaptiveHandlerWrapper

#[derive(Clone)]
struct AdaptiveHandlerWrapper<L>(L);

impl<L> Layer<AdaptiveHandler> for AdaptiveHandlerWrapper<L>
where
	L: Layer<AdaptiveHandler> + Clone,
	L::Handler:
		Handler<Response = Response, Future = BoxedFuture<Response>> + Clone + Send + Sync + 'static,
{
	type Handler = BoxedHandler;

	fn wrap(&self, handler: AdaptiveHandler) -> Self::Handler {
		BoxedHandler::new(self.0.wrap(handler))
	}
}
