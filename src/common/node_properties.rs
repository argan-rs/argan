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

/// A type that represents the *cookie key* as a property.
#[cfg(any(feature = "private-cookies", feature = "signed-cookies"))]
pub struct NodeCookieKey;

#[cfg(any(feature = "private-cookies", feature = "signed-cookies"))]
impl NodeCookieKey {
	/// Passes the given key as a property value to a node.
	pub fn to<K, Mark>(self, key: K) -> NodeProperty<Mark>
	where
		K: Into<cookie::Key>,
	{
		NodeProperty::CookieKey(key.into())
	}
}

// --------------------------------------------------
// RequestExtensionsModifier

/// A type that represents the *request extensions modifier* middleware as a property.
pub struct RequestExtensionsModifier;

impl RequestExtensionsModifier {
	/// Passes the given function as a property value to a node.
	pub fn to<Func, Mark>(self, modifier: Func) -> NodeProperty<Mark>
	where
		Func: Fn(&mut Extensions) + Clone + Send + Sync + 'static,
	{
		let request_extensions_modifier_layer = RequestExtensionsModifierLayer::new(modifier);

		NodeProperty::<Mark>::RequestExtensionsModifier(request_extensions_modifier_layer)
	}
}

// --------------------------------------------------------------------------------
