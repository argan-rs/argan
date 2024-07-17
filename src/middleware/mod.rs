//! Middleware system types and traits.

// ----------

use argan_core::BoxedFuture;
use tower_layer::Layer as TowerLayer;

use crate::{
	handler::{AdaptiveHandler, BoxedHandler, Handler, HandlerService},
	response::{BoxedErrorResponse, IntoResponse, Response},
};

// --------------------------------------------------

mod impls;
pub use impls::*;

pub(crate) mod layer_stack;

pub(crate) mod targets;
pub use targets::{HandlerWrapper, RequestHandler, RequestPasser, RequestReceiver};

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

/// Implemented by types that apply middleware to a [`Handler`];
pub trait Layer<H> {
	type Handler;

	/// Wraps the `handler` with the `Layer`'s middleware.
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

/// A trait for types that can be converted into a [`Layer`].
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

impl<Func, InH, OutH> IntoLayer<Func, InH> for Func
where
	Func: Fn(InH) -> OutH,
{
	type Layer = LayerFn<Func>;

	fn into_layer(self) -> Self::Layer {
		LayerFn(self)
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
		L::Handler: Handler<
				Response = Response,
				Error = BoxedErrorResponse,
				Future = BoxedFuture<Result<Response, BoxedErrorResponse>>,
			> + Clone
			+ Send
			+ Sync
			+ 'static,
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
	L::Handler: Handler<
			Response = Response,
			Error = BoxedErrorResponse,
			Future = BoxedFuture<Result<Response, BoxedErrorResponse>>,
		> + Clone
		+ Send
		+ Sync
		+ 'static,
{
	type Handler = BoxedHandler;

	fn wrap(&self, handler: AdaptiveHandler) -> Self::Handler {
		BoxedHandler::new(self.0.wrap(handler))
	}
}

// --------------------------------------------------------------------------------
