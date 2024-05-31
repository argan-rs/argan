//! Fundamental HTTP types.

pub use argan_core::http::*;

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

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
