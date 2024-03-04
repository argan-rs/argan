use http::Method;

use crate::{
	common::{BoxedError, BoxedFuture, IntoArray},
	handler::{AdaptiveHandler, Handler},
	middleware::{BoxedLayer, IntoLayer, Layer},
	response::{BoxedErrorResponse, Response},
};

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

mod private {
	use super::*;

	#[allow(private_interfaces)]
	#[derive(Default)]
	pub enum ResourceLayerTarget {
		#[default]
		None,
		RequestReceiver(BoxedLayer),
		RequestPasser(BoxedLayer),
		RequestHandler(BoxedLayer),
		MethodHandler(Vec<Method>, BoxedLayer),
		WildcardMethodHandler(BoxedLayer),
		MistargetedRequestHandler(BoxedLayer),
	}

	impl ResourceLayerTarget {
		#[inline(always)]
		pub(crate) fn take(&mut self) -> Self {
			std::mem::take(self)
		}
	}
}

pub(crate) use private::ResourceLayerTarget;

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
			ResourceLayerTarget::$kind(BoxedLayer::new(layer.into_layer()))
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
	ResourceLayerTarget::MethodHandler(
		methods.into_array().into(),
		BoxedLayer::new(layer.into_layer()),
	)
}

layer_target_wrapper!(wildcard_method_handler, WildcardMethodHandler);

layer_target_wrapper!(mistargeted_request_handler, MistargetedRequestHandler);

// --------------------------------------------------------------------------------
