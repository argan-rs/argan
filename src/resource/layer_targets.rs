use http::Method;

use crate::{
	common::{BoxedError, BoxedFuture, IntoArray},
	handler::{AdaptiveHandler, Handler},
	middleware::{BoxedLayer, IntoLayer, Layer},
	response::{BoxedErrorResponse, Response},
};

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

pub struct ResourceLayerTarget(pub(crate) ResourceLayerTargetValue);

#[derive(Default)]
pub(crate) enum ResourceLayerTargetValue {
	#[default]
	None,
	RequestReceiver(BoxedLayer),
	RequestPasser(BoxedLayer),
	RequestHandler(BoxedLayer),
	MethodHandler(Vec<Method>, BoxedLayer),
	WildcardMethodHandler(BoxedLayer),
	MistargetedRequestHandler(BoxedLayer),
}

impl ResourceLayerTargetValue {
	#[inline(always)]
	pub(crate) fn take(&mut self) -> Self {
		std::mem::take(self)
	}
}

// ----------

macro_rules! layer_target_wrapper {
	($func:ident, $kind:ident) => {
		pub fn $func<L, M>(layer: L) -> ResourceLayerTarget
		where
			L: IntoLayer<M, AdaptiveHandler>,
			L::Layer: Layer<AdaptiveHandler> + Clone + 'static,
			<L::Layer as Layer<AdaptiveHandler>>::Handler: Handler<
					Response = Response,
					Error = BoxedErrorResponse,
					Future = BoxedFuture<Result<Response, BoxedErrorResponse>>,
				> + Clone
				+ Send
				+ Sync
				+ 'static,
		{
			ResourceLayerTarget(ResourceLayerTargetValue::$kind(BoxedLayer::new(
				layer.into_layer(),
			)))
		}
	};
}

layer_target_wrapper!(request_receiver, RequestReceiver);

layer_target_wrapper!(request_passer, RequestPasser);

layer_target_wrapper!(request_handler, RequestHandler);

pub fn method_handler<M, const N: usize, L, Mark>(methods: M, layer: L) -> ResourceLayerTarget
where
	M: IntoArray<Method, N>,
	L: IntoLayer<Mark, AdaptiveHandler>,
	L::Layer: Layer<AdaptiveHandler> + Clone + 'static,
	<L::Layer as Layer<AdaptiveHandler>>::Handler: Handler<
			Response = Response,
			Error = BoxedErrorResponse,
			Future = BoxedFuture<Result<Response, BoxedErrorResponse>>,
		> + Clone
		+ Send
		+ Sync
		+ 'static,
{
	ResourceLayerTarget(ResourceLayerTargetValue::MethodHandler(
		methods.into_array().into(),
		BoxedLayer::new(layer.into_layer()),
	))
}

layer_target_wrapper!(wildcard_method_handler, WildcardMethodHandler);

layer_target_wrapper!(mistargeted_request_handler, MistargetedRequestHandler);

// --------------------------------------------------------------------------------
