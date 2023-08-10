use tower_layer::Layer as TowerLayer;

use crate::handler::{Handler, HandlerService};

// -------------------------

mod internal;

pub(crate) use internal::*;

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

pub trait Layer<H, OuterB, LayeredB> {
	type Handler;

	fn wrap(&self, handler: H) -> Self::Handler;
}

impl<L, H, OuterB, LayeredB> Layer<H, OuterB, LayeredB> for L
where
	L: TowerLayer<HandlerService<H, LayeredB>>,
	L::Service: Handler<OuterB>,
{
	type Handler = L::Service;

	fn wrap(&self, handler: H) -> Self::Handler {
		self.layer(HandlerService::from(handler))
	}
}

// --------------------------------------------------------------------------------
