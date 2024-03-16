use crate::middleware::RequestExtensionsModifierLayer;

use super::*;

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

bit_flags! {
	#[derive(Clone)]
	pub(super) ConfigFlags: u8 {
		pub(super) ENDS_WITH_SLASH = 0b0001;
		pub(super) REDIRECTS_ON_UNMATCHING_SLASH = 0b0010;
		pub(super) DROPS_ON_UNMATCHING_SLASH = 0b0100;
		pub(super) SUBTREE_HANDLER = 0b1000;
	}
}

impl Display for ConfigFlags {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		let mut flags = String::new();

		if self.has(Self::ENDS_WITH_SLASH) {
			flags.push_str("ends_with_slash");
		}

		if self.has(Self::REDIRECTS_ON_UNMATCHING_SLASH) {
			if !flags.is_empty() {
				flags.push_str(", ")
			}

			flags.push_str("redirects_on_unmatching_slash");
		} else if self.has(Self::DROPS_ON_UNMATCHING_SLASH) {
			if !flags.is_empty() {
				flags.push_str(", ")
			}

			flags.push_str("drops_on_unmatching_slash");
		} else {
			if !flags.is_empty() {
				flags.push_str(", ")
			}

			flags.push_str("handles_on_unmatching_slash");
		}

		if self.has(Self::SUBTREE_HANDLER) {
			if !flags.is_empty() {
				flags.push_str(", ")
			}

			flags.push_str("subtree_handler");
		}

		f.write_str(&flags)
	}
}

impl Default for ConfigFlags {
	fn default() -> Self {
		let mut flags = Self(0);
		flags.add(Self::REDIRECTS_ON_UNMATCHING_SLASH);

		flags
	}
}

// --------------------------------------------------
// ResourceConfigOption

config_option! {
	ResourceConfigOption {
		DropOnUnmatchingSlash,
		HandleOnUnmatchingSlash,
		HandleSubtreeRequests,
		RequestExtensionsModifier(RequestExtensionsModifierLayer),
	}
}

// ----------

pub fn _to_drop_on_unmatching_slash() -> ResourceConfigOption {
	ResourceConfigOption::DropOnUnmatchingSlash
}

pub fn _to_handle_on_unmatching_slash() -> ResourceConfigOption {
	ResourceConfigOption::HandleOnUnmatchingSlash
}

pub fn _to_handle_subtree_requests() -> ResourceConfigOption {
	ResourceConfigOption::HandleSubtreeRequests
}

pub fn _with_request_extensions_modifier<Func>(modifier: Func) -> ResourceConfigOption
where
	Func: Fn(&mut Extensions) + Clone + Send + Sync + 'static,
{
	let request_extensions_modifier_layer = RequestExtensionsModifierLayer::new(modifier);

	ResourceConfigOption::RequestExtensionsModifier(request_extensions_modifier_layer)
}

// --------------------------------------------------------------------------------
