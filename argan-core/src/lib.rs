use std::{future::Future, pin::Pin};

// ----------

pub(crate) use std::error::Error as StdError;
pub(crate) use thiserror::Error as ImplError;

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

#[macro_use]
pub(crate) mod macros;

pub mod body;
pub mod http;
pub mod request;
pub mod response;

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

pub type BoxedError = Box<dyn StdError + Send + Sync>;
pub type BoxedFuture<T> = Pin<Box<dyn Future<Output = T> + Send>>;

// --------------------------------------------------------------------------------

// --------------------------------------------------
// Marker

pub(crate) mod marker {
	pub struct Private;
}

// --------------------------------------------------
// Used when expecting a valid value in Options or Results.
pub(crate) const SCOPE_VALIDITY: &'static str = "scope validity";

// --------------------------------------------------------------------------------
