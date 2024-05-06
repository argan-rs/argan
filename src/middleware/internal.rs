use std::{
	fmt::Debug,
	future::Future,
	pin::Pin,
	task::{Context, Poll},
};

use argan_core::{request::Request, BoxedFuture};
use bytes::Bytes;
use http::Extensions;
use http_body_util::BodyExt;
use pin_project::pin_project;

use crate::{
	handler::{AdaptiveHandler, Args, BoxedHandler, DummyHandler, Handler},
	request::RequestContext,
	response::{BoxedErrorResponse, IntoResponse, Response},
};

use super::Layer;

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

// --------------------------------------------------
// LayerFn

pub struct LayerFn<Func>(pub(crate) Func);

impl<Func, InH, OutH> Layer<InH> for LayerFn<Func>
where
	Func: Fn(InH) -> OutH,
{
	type Handler = OutH;

	fn wrap(&self, handler: InH) -> Self::Handler {
		self.0(handler)
	}
}

// --------------------------------------------------------------------------------

#[derive(Clone)]
pub(crate) struct RequestExtensionsModifierLayer(BoxedExtensionsModifier);

impl RequestExtensionsModifierLayer {
	pub(crate) fn new<Func>(modifier: Func) -> Self
	where
		Func: Fn(&mut Extensions) + Clone + Send + Sync + 'static,
	{
		Self(BoxedExtensionsModifier::new(modifier))
	}
}

impl Layer<AdaptiveHandler> for RequestExtensionsModifierLayer {
	type Handler = RequestExtensionsModifier<AdaptiveHandler>;

	fn wrap(&self, handler: AdaptiveHandler) -> Self::Handler {
		RequestExtensionsModifier::new(handler, self.0.clone())
	}
}

// ----------

#[derive(Clone)]
pub(crate) struct RequestExtensionsModifier<H> {
	inner_handler: H,
	boxed_modifier: BoxedExtensionsModifier,
}

impl<H> RequestExtensionsModifier<H> {
	fn new(handler: H, boxed_modifier: BoxedExtensionsModifier) -> Self {
		Self {
			inner_handler: handler,
			boxed_modifier,
		}
	}
}

impl<H, B> Handler<B> for RequestExtensionsModifier<H>
where
	H: Handler<B>,
{
	type Response = H::Response;
	type Error = H::Error;
	type Future = H::Future;

	#[inline(always)]
	fn handle(&self, mut request_context: RequestContext<B>, args: Args<'_, ()>) -> Self::Future {
		self.boxed_modifier.0(request_context.request_mut().extensions_mut());

		self.inner_handler.handle(request_context, args)
	}
}

// -------------------------

trait ExtensionsModifier: Fn(&mut Extensions) {
	fn boxed_clone(&self) -> BoxedExtensionsModifier;
}

impl<Func> ExtensionsModifier for Func
where
	Func: Fn(&mut Extensions) + Clone + Send + Sync + 'static,
{
	fn boxed_clone(&self) -> BoxedExtensionsModifier {
		BoxedExtensionsModifier::new(self.clone())
	}
}

// -------------------------

struct BoxedExtensionsModifier(Box<dyn ExtensionsModifier + Send + Sync + 'static>);

impl BoxedExtensionsModifier {
	pub(crate) fn new<Func>(modifier: Func) -> Self
	where
		Func: Fn(&mut Extensions) + Clone + Send + Sync + 'static,
	{
		Self(Box::new(modifier))
	}
}

impl Clone for BoxedExtensionsModifier {
	fn clone(&self) -> Self {
		self.0.boxed_clone()
	}
}

// --------------------------------------------------------------------------------
