use tower_layer::Layer as TowerLayer;

use crate::handler::HandlerService;

// --------------------------------------------------

mod internal;

pub(crate) use internal::*;

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

pub trait Layer<H, LayeredB> {
	type Handler;

	fn wrap(&self, handler: H) -> Self::Handler;
}

impl<L, H, LayeredB> Layer<H, LayeredB> for L
where
	L: TowerLayer<HandlerService<H, LayeredB>>,
{
	type Handler = L::Service;

	fn wrap(&self, handler: H) -> Self::Handler {
		self.layer(HandlerService::from(handler))
	}
}

// --------------------------------------------------------------------------------
