use std::sync::Arc;

use http::Method;
use tower_layer::Layer as TowerLayer;

use crate::{
	common::IntoArray,
	handler::{AdaptiveHandler, BoxedHandler, FinalHandler, Handler, HandlerService},
	response::IntoResponse,
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

pub(crate) trait FinalLayer
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
		Box::new(self)
	}

	fn boxed_clone(&self) -> BoxedLayer {
		Box::new(self.clone())
	}
}

// --------------------------------------------------

pub(crate) type BoxedLayer = Box<dyn FinalLayer>;

// -------------------------

pub struct LayerTarget(pub(crate) Inner);

pub(crate) enum Inner {
	RequestReceiver(BoxedLayer),
	RequestPasser(BoxedLayer),
	RequestHandler(BoxedLayer),
	MethodHandler(Vec<Method>, BoxedLayer),
	AllMethodsHandler(BoxedLayer),
	MisdirectedRequestHandler(BoxedLayer),
}

trait IntoLayerTargetList<const N: usize> {
	fn into_layer_kind_list(self) -> [LayerTarget; N];
}

// ----------

macro_rules! layer_target_wrapper {
	($func:ident, $kind:ident) => {
		pub fn $func<L, M>(layer: L) -> LayerTarget
		where
			L: IntoLayer<M, AdaptiveHandler>,
			L::Layer: Layer<AdaptiveHandler> + Clone + 'static,
			<L::Layer as Layer<AdaptiveHandler>>::Handler: Handler + Clone + Send + Sync + 'static,
			<<L::Layer as Layer<AdaptiveHandler>>::Handler as Handler>::Response: IntoResponse,
		{
			LayerTarget(Inner::$kind(Box::new(AdaptiveHandlerWrapper(
				layer.into_layer(),
			))))
		}
	};
}

layer_target_wrapper!(request_receiver_with, RequestReceiver);

layer_target_wrapper!(request_passer_with, RequestPasser);

layer_target_wrapper!(request_handler_with, RequestHandler);

pub fn method_handler_of<M, const N: usize, L, Mark>(methods: M, layer: L) -> LayerTarget
where
	M: IntoArray<Method, N>,
	L: IntoLayer<Mark, AdaptiveHandler>,
	L::Layer: Layer<AdaptiveHandler> + Clone + 'static,
	<L::Layer as Layer<AdaptiveHandler>>::Handler: Handler + Clone + Send + Sync + 'static,
	<<L::Layer as Layer<AdaptiveHandler>>::Handler as Handler>::Response: IntoResponse,
{
	LayerTarget(Inner::MethodHandler(
		methods.into_array().into(),
		AdaptiveHandlerWrapper(layer.into_layer()).into_boxed_layer(),
	))
}

layer_target_wrapper!(all_methods_handler_with, AllMethodsHandler);

layer_target_wrapper!(misdirected_request_handler_with, MisdirectedRequestHandler);

// ----------

#[derive(Clone)]
struct AdaptiveHandlerWrapper<L>(L);

impl<L> Layer<AdaptiveHandler> for AdaptiveHandlerWrapper<L>
where
	L: Layer<AdaptiveHandler> + Clone,
	L::Handler: Handler + Clone + Send + Sync + 'static,
	<L::Handler as Handler>::Response: IntoResponse,
{
	type Handler = BoxedHandler;

	fn wrap(&self, handler: AdaptiveHandler) -> Self::Handler {
		let layered_handler = self.0.wrap(handler);
		let ready_handler = ResponseFutureBoxer::wrap(IntoResponseAdapter::wrap(layered_handler));

		ready_handler.into_boxed_handler()
	}
}
