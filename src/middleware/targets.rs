//! Layer targets to apply middleware.

// ----------

use std::str::FromStr;

use http::Method;

use crate::{
	common::{marker::Sealed, IntoArray},
	handler::{AdaptiveHandler, BoxableHandler},
	http::{CustomMethod, WildcardMethod},
	middleware::{BoxedLayer, IntoLayer, Layer},
	request::MistargetedRequest,
	resource::Resource,
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
		MethodHandler(Method, BoxedLayer),
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

// --------------------------------------------------

/// A trait that's implemented by the [`Method`] type to pass
/// the given `layer` to wrap the *method* handler.
pub trait HandlerWrapper: Sealed {
	fn handler_in<L, Mark>(self, layer: L) -> LayerTarget<Resource>
	where
		L: IntoLayer<Mark, AdaptiveHandler>,
		L::Layer: Layer<AdaptiveHandler> + Clone + 'static,
		<L::Layer as Layer<AdaptiveHandler>>::Handler: BoxableHandler + Clone + Send + Sync + 'static;
}

impl HandlerWrapper for Method {
	fn handler_in<L, Mark>(self, layer: L) -> LayerTarget<Resource>
	where
		L: IntoLayer<Mark, AdaptiveHandler>,
		L::Layer: Layer<AdaptiveHandler> + Clone + 'static,
		<L::Layer as Layer<AdaptiveHandler>>::Handler: BoxableHandler + Clone + Send + Sync + 'static,
	{
		LayerTarget::MethodHandler(self, BoxedLayer::new(layer.into_layer()))
	}
}

// --------------------------------------------------
// CustomMethod

impl<M: AsRef<str>> CustomMethod<M> {
	/// Passes the `layer` to wrap the *custom HTTP method* handler.
	pub fn handler_in<L, Mark>(self, layer: L) -> LayerTarget<Resource>
	where
		L: IntoLayer<Mark, AdaptiveHandler>,
		L::Layer: Layer<AdaptiveHandler> + Clone + 'static,
		<L::Layer as Layer<AdaptiveHandler>>::Handler: BoxableHandler + Clone + Send + Sync + 'static,
	{
		let method = Method::from_str(self.0.as_ref())
			.expect("HTTP method should be a valid token [RFC 9110, 5.6.2 Tokens]");

		LayerTarget::MethodHandler(method, BoxedLayer::new(layer.into_layer()))
	}
}

// --------------------------------------------------
// WildcardMethod

impl WildcardMethod {
	/// Passes the `layer` to wrap the *wildcard method* handler.
	pub fn handler_in<L, Mark>(self, layer: L) -> LayerTarget<Resource>
	where
		L: IntoLayer<Mark, AdaptiveHandler>,
		L::Layer: Layer<AdaptiveHandler> + Clone + 'static,
		<L::Layer as Layer<AdaptiveHandler>>::Handler: BoxableHandler + Clone + Send + Sync + 'static,
	{
		LayerTarget::WildcardMethodHandler(BoxedLayer::new(layer.into_layer()))
	}
}

// --------------------------------------------------
// MistargetedRequest

impl MistargetedRequest {
	/// Passes the `layer` to wrap the *mistargeted request* handler.
	pub fn handler_in<L, Mark>(self, layer: L) -> LayerTarget<Resource>
	where
		L: IntoLayer<Mark, AdaptiveHandler>,
		L::Layer: Layer<AdaptiveHandler> + Clone + 'static,
		<L::Layer as Layer<AdaptiveHandler>>::Handler: BoxableHandler + Clone + Send + Sync + 'static,
	{
		LayerTarget::MistargetedRequestHandler(BoxedLayer::new(layer.into_layer()))
	}
}

// --------------------------------------------------

macro_rules! layer_target_wrapper {
	($target:ident, #[$struct_comment:meta], #[$method_comment:meta]$(,)?) => {
		#[$struct_comment]
		pub struct $target;

		impl $target {
			pub fn component_in<L, Mark>(self, layer: L) -> LayerTarget<Resource>
			where
				L: IntoLayer<Mark, AdaptiveHandler>,
				L::Layer: Layer<AdaptiveHandler> + Clone + 'static,
				<L::Layer as Layer<AdaptiveHandler>>::Handler:
					BoxableHandler + Clone + Send + Sync + 'static,
			{
				LayerTarget::$target(BoxedLayer::new(layer.into_layer()))
			}
		}
	};
}

// --------------------------------------------------
// RequestReceiver

layer_target_wrapper!(
	RequestReceiver,
	#[doc = "A type that represents the *request receiver*."],
	#[doc = "Passes the `layer` to wrap the *request receiver*."],
);

// --------------------------------------------------
// RequestPasser

/// A type that represents the *request passer*.
pub struct RequestPasser;

impl RequestPasser {
	/// Passes the `layer` to wrap the *request passer*.
	pub fn component_in<TargetMark, L, Mark>(self, layer: L) -> LayerTarget<TargetMark>
	where
		L: IntoLayer<Mark, AdaptiveHandler>,
		L::Layer: Layer<AdaptiveHandler> + Clone + 'static,
		<L::Layer as Layer<AdaptiveHandler>>::Handler: BoxableHandler + Clone + Send + Sync + 'static,
	{
		LayerTarget::RequestPasser(BoxedLayer::new(layer.into_layer()))
	}
}

// --------------------------------------------------
// RequestHandler

layer_target_wrapper!(
	RequestHandler,
	#[doc = "A type that represents the *request handler*."],
	#[doc = "Passes the `layer` to wrap the *request handler*."],
);

// --------------------------------------------------------------------------------
