//! Configuration options for nodes ([`Router`](crate::Router), [`Resource`](crate::Resource));

// ----------

use http::Extensions;

use crate::middleware::RequestExtensionsModifierLayer;

use super::*;

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

// --------------------------------------------------
// ResourceConfigOption

option! {
	pub(crate) ConfigOption<Mark> {
		#[cfg(any(feature = "private-cookies", feature = "signed-cookies"))]
		CookieKey(cookie::Key),
		RequestExtensionsModifier(RequestExtensionsModifierLayer),
	}
}

// ----------

/// Passes the given [cookie::Key] as a config option for a node.
#[cfg(any(feature = "private-cookies", feature = "signed-cookies"))]
pub fn _with_cookie_key<Mark>(cookie_key: cookie::Key) -> ConfigOption<Mark> {
	ConfigOption::<Mark>::CookieKey(cookie_key)
}

// ----------

/// Passes the given 'extensions modifier' function as a config option for a node.
pub fn _with_request_extensions_modifier<Mark, Func>(modifier: Func) -> ConfigOption<Mark>
where
	Func: Fn(&mut Extensions) + Clone + Send + Sync + 'static,
{
	let request_extensions_modifier_layer = RequestExtensionsModifierLayer::new(modifier);

	ConfigOption::<Mark>::RequestExtensionsModifier(request_extensions_modifier_layer)
}

// --------------------------------------------------------------------------------
