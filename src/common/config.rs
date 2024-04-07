use http::Extensions;

use crate::middleware::RequestExtensionsModifierLayer;

use super::*;

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

// --------------------------------------------------
// ResourceConfigOption

option! {
	pub(crate) ConfigOption<Mark> {
		CookieKey(cookie::Key),
		RequestExtensionsModifier(RequestExtensionsModifierLayer),
	}
}

// ----------

pub fn _with_cookie_key<Mark>(cookie_key: cookie::Key) -> ConfigOption<Mark> {
	ConfigOption::<Mark>::CookieKey(cookie_key)
}

// ----------

pub fn _with_request_extensions_modifier<Mark, Func>(modifier: Func) -> ConfigOption<Mark>
where
	Func: Fn(&mut Extensions) + Clone + Send + Sync + 'static,
{
	let request_extensions_modifier_layer = RequestExtensionsModifierLayer::new(modifier);

	ConfigOption::<Mark>::RequestExtensionsModifier(request_extensions_modifier_layer)
}

// --------------------------------------------------------------------------------
