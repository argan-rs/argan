//! Layer targets to apply middleware.

// ----------

use argan_core::BoxedFuture;
use http::Method;

use crate::{
	common::IntoArray,
	handler::{AdaptiveHandler, Handler},
	middleware::{BoxedLayer, IntoLayer, Layer},
	resource::Resource,
	response::{BoxedErrorResponse, Response},
};

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

mod private {
	use std::marker::PhantomData;

	use super::*;

	#[allow(private_interfaces)]
	pub enum LayerTarget<Mark> {
		None(PhantomData<fn() -> Mark>),
		RequestReceiver(BoxedLayer),
		RequestPasser(BoxedLayer),
		RequestHandler(BoxedLayer),
		MethodHandler(Vec<Method>, BoxedLayer),
		WildcardMethodHandler(BoxedLayer),
		MistargetedRequestHandler(BoxedLayer),
	}

	impl<Mark> LayerTarget<Mark> {
		#[inline(always)]
		pub(crate) fn take(&mut self) -> Self {
			std::mem::take(self)
		}
	}

	impl<Mark> Default for LayerTarget<Mark> {
		fn default() -> Self {
			Self::None(PhantomData)
		}
	}

	impl<Mark> IntoArray<LayerTarget<Mark>, 1> for LayerTarget<Mark> {
		fn into_array(self) -> [LayerTarget<Mark>; 1] {
			[self]
		}
	}
}

pub(crate) use private::LayerTarget;

// ----------

macro_rules! layer_target_wrapper {
	($func:ident, $kind:ident, #[$comment:meta]) => {
		#[$comment]
		pub fn $func<L, Mark>(layer: L) -> LayerTarget<Resource>
		where
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
			LayerTarget::$kind(BoxedLayer::new(layer.into_layer()))
		}
	};
}

layer_target_wrapper!(
	_request_receiver,
	RequestReceiver,
	#[doc = "Passes the layer to be applied to a *request receiver*."]
);

/// Passes the layer to be applied to a *request passer*.
pub fn _request_passer<TargetMark, L, Mark>(layer: L) -> LayerTarget<TargetMark>
where
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
	LayerTarget::RequestPasser(BoxedLayer::new(layer.into_layer()))
}

layer_target_wrapper!(
	_request_handler,
	RequestHandler,
	#[doc = "Passes the layer to be applied to a *request handler*."]
);

/// Passes the layer to be applied to specified *method handlers*.
pub fn _method_handler<M, const N: usize, L, Mark>(methods: M, layer: L) -> LayerTarget<Resource>
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
	LayerTarget::MethodHandler(
		methods.into_array().into(),
		BoxedLayer::new(layer.into_layer()),
	)
}

layer_target_wrapper!(
	_wildcard_method_handler,
	WildcardMethodHandler,
	#[doc = "Passes the layer to be applied to a *wildcard method handler*."]
);

layer_target_wrapper!(
	_mistargeted_request_handler,
	MistargetedRequestHandler,
	#[doc = "Passes the layer to be applied to a *mistargeted request handler*."]
);

// --------------------------------------------------------------------------------
