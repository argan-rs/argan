use http::Method;
use tower_layer::Layer as TowerLayer;

use crate::{
	handler::{AdaptiveHandler, ArcHandler, Handler, HandlerService, IntoArcHandler},
	response::IntoResponse,
	utils::mark::Private,
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

pub(crate) type BoxedLayer = Box<dyn Layer<AdaptiveHandler, Handler = ArcHandler>>;

// -------------------------

pub struct LayerTarget(pub(crate) Inner);

pub(crate) enum Inner {
	RequestReceiver(BoxedLayer),
	RequestPasser(BoxedLayer),
	RequestHandler(BoxedLayer),
	MethodHandler(Method, BoxedLayer),
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
			L::Layer: Layer<AdaptiveHandler> + 'static,
			<L::Layer as Layer<AdaptiveHandler>>::Handler: Handler + Send + Sync + 'static,
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

pub fn method_handler_of<L, M>(method: Method, layer: L) -> LayerTarget
where
	L: IntoLayer<M, AdaptiveHandler>,
	L::Layer: Layer<AdaptiveHandler> + 'static,
	<L::Layer as Layer<AdaptiveHandler>>::Handler: Handler + Send + Sync + 'static,
	<<L::Layer as Layer<AdaptiveHandler>>::Handler as Handler>::Response: IntoResponse,
{
	LayerTarget(Inner::MethodHandler(
		method,
		Box::new(AdaptiveHandlerWrapper(layer.into_layer())),
	))
}

layer_target_wrapper!(all_methods_handler_with, AllMethodsHandler);

layer_target_wrapper!(misdirected_request_handler_with, MisdirectedRequestHandler);

// ----------

struct AdaptiveHandlerWrapper<L>(L);

impl<L> Layer<AdaptiveHandler> for AdaptiveHandlerWrapper<L>
where
	L: Layer<AdaptiveHandler>,
	L::Handler: Handler + Send + Sync + 'static,
	<L::Handler as Handler>::Response: IntoResponse,
{
	type Handler = ArcHandler;

	fn wrap(&self, handler: AdaptiveHandler) -> Self::Handler {
		let layered_handler = self.0.wrap(handler);
		let ready_handler = ResponseFutureBoxer::wrap(IntoResponseAdapter::wrap(layered_handler));

		ready_handler.into_arc_handler()
	}
}
