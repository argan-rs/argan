//! Optional properties of a node ([`Router`](crate::Router), [`Resource`](crate::Resource)).

// ----------

use http::Extensions;

use crate::middleware::RequestExtensionsModifierLayer;

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

// --------------------------------------------------
// Property

option! {
	pub(crate) NodeProperty<Mark> {
		#[cfg(any(feature = "private-cookies", feature = "signed-cookies"))]
		CookieKey(cookie::Key),
		RequestExtensionsModifier(RequestExtensionsModifierLayer),
	}
}

// --------------------------------------------------
// CookieKey

/// A type that represents a *cookie key* as a property.
#[cfg(any(feature = "private-cookies", feature = "signed-cookies"))]
pub struct NodeCookieKey;

#[cfg(any(feature = "private-cookies", feature = "signed-cookies"))]
impl NodeCookieKey {
	/// Passes the cryptographic `Key` used for *private* and *signed* cookies
	/// as a node property.
	pub fn to<K, Mark>(self, key: K) -> NodeProperty<Mark>
	where
		K: Into<cookie::Key>,
	{
		NodeProperty::CookieKey(key.into())
	}
}

// --------------------------------------------------
// RequestExtensionsModifier

/// A type that represents *request extensions modifier* middleware as a property.
pub struct RequestExtensionsModifier;

impl RequestExtensionsModifier {
	/// Passes the given function as a property to a node.
	pub fn to<Func, Mark>(self, modifier: Func) -> NodeProperty<Mark>
	where
		Func: Fn(&mut Extensions) + Clone + Send + Sync + 'static,
	{
		let request_extensions_modifier_layer = RequestExtensionsModifierLayer::new(modifier);

		NodeProperty::<Mark>::RequestExtensionsModifier(request_extensions_modifier_layer)
	}
}

// --------------------------------------------------------------------------------
