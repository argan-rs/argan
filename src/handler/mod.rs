pub use hyper::service::Service;

use crate::{body::Incoming, request::Request};

// --------------------------------------------------
pub mod implementation;

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

pub trait Handler<RqB = Incoming>
where
	Self: Service<Request<RqB>> + Clone + Send,
{
}

impl<S, RqB> Handler<RqB> for S where S: Service<Request<RqB>> + Clone + Send {}

// -------------------------

pub trait IntoHandler<H, RqB = Incoming>
where
	H: Handler<RqB>,
{
	fn into_handler(self) -> H;
}

impl<H, RqB> IntoHandler<H, RqB> for H
where
	H: Handler<RqB>,
{
	#[inline]
	fn into_handler(self) -> H {
		self
	}
}

