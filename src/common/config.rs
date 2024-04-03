use http::Extensions;

use crate::middleware::RequestExtensionsModifierLayer;

use super::*;

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

// --------------------------------------------------
// ResourceConfigOption

config_option! {
	ConfigOption<Mark> {
		CookieKey(cookie::Key),
		RequestExtensionsModifier(RequestExtensionsModifierLayer),
	}
}

pub(crate) use config_private::ConfigOption;

// ----------

pub fn _cookie_key<Mark, K>(cookie_key: K) -> ConfigOption<Mark>
where
	K: for<'k> TryFrom<&'k [u8]> + Into<cookie::Key>,
{
	ConfigOption::<Mark>::CookieKey(cookie_key.into())
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
