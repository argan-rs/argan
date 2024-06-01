//! Fundamental HTTP types.

pub use argan_core::http::*;

use crate::common::{marker::Sealed, IntoArray};

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

// --------------------------------------------------
// Method

impl IntoArray<Method, 1> for Method {
	fn into_array(self) -> [Method; 1] {
		[self]
	}
}

impl Sealed for Method {}

// --------------------------------------------------
// CustomMethod

/// A type that represents a *custom HTTP method*.
///
/// ```
/// use argan::{Resource, http::CustomMethod};
///
/// let mut resource = Resource::new("/");
/// resource.set_handler_for(CustomMethod("LOCK").to(|| async { /* ... */ }));
/// ```
pub struct CustomMethod<M>(pub M);

// --------------------------------------------------
// WildcardMethod

/// A type that represents a *wildcard method*.
///
/// ```
/// use argan::{Resource, http::WildcardMethod};
///
/// let mut resource = Resource::new("/");
/// resource.set_handler_for(WildcardMethod.to(Some(|| async { /* ... */ })));
/// ```
pub struct WildcardMethod;

// --------------------------------------------------
