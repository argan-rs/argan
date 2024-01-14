use tower_layer::Layer as TowerLayer;

use crate::handler::{HandlerService, AdaptiveHandler};

// --------------------------------------------------

mod internal;

pub(crate) use internal::*;

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

pub trait Layer<H> {
	type Handler;

	fn wrap(&self, handler: H) -> Self::Handler;
}

impl<L, H> Layer<H> for L
where
	L: TowerLayer<HandlerService<H>>,
{
	type Handler = L::Service;

	fn wrap(&self, handler: H) -> Self::Handler {
		self.layer(HandlerService::from(handler))
	}
}

// --------------------------------------------------

// type BoxedLayer = Box<dyn Layer<AdaptiveHandler>>;
