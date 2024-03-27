use std::{future::Future, pin::Pin};

// ----------

pub use std::error::Error as StdError;

pub(crate) use thiserror::Error as ImplError;

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

#[macro_use]
pub(crate) mod macros;

pub mod body;
pub mod extensions;
pub mod request;
pub mod response;

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

pub type BoxedError = Box<dyn StdError + Send + Sync>;
pub type BoxedFuture<T> = Pin<Box<dyn Future<Output = T> + Send>>;

// --------------------------------------------------------------------------------

// --------------------------------------------------
// Arguments

pub trait Arguments<'n, HandlerExt = ()>: Sized {
	fn private_extension(&mut self) -> &mut impl PrivateType;
	fn node_extension<Ext: Send + Sync + 'static>(&self) -> Option<&'n Ext>;
	fn handler_extension(&self) -> &'n HandlerExt;
}

// --------------------------------------------------
// PrivateType marker

pub trait PrivateType {}

// --------------------------------------------------
// IntoArray trait

pub trait IntoArray<T, const N: usize> {
	fn into_array(self) -> [T; N];
}

impl<T, const N: usize> IntoArray<T, N> for [T; N]
where
	T: IntoArray<T, 1>,
{
	fn into_array(self) -> [T; N] {
		self
	}
}

// --------------------------------------------------
// Marker

pub(crate) mod marker {
	pub struct Private;
}

// --------------------------------------------------
// Used when expecting a valid value in Options or Results.
pub(crate) const SCOPE_VALIDITY: &'static str = "scope validity";

// --------------------------------------------------------------------------------
