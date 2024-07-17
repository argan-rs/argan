//! Optional properties of a node ([`Router`](crate::Router), [`Resource`](crate::Resource)).

// ----------

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

// --------------------------------------------------
// Property

option! {
	pub(crate) NodeProperty<Mark> {
		#[cfg(any(feature = "private-cookies", feature = "signed-cookies"))]
		CookieKey(cookie::Key),
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

// --------------------------------------------------------------------------------
