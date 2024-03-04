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

// ----------

mod private {
	use super::*;

	#[allow(private_interfaces)]
	pub enum ResourceConfigOption {
		DropOnUnmatchingSlash,
		HandleOnUnmatchingSlash,
		SubtreeHandler,
		ModifyRequestExtensions(RequestExtensionsModifierLayer),
	}

	impl IntoArray<ResourceConfigOption, 1> for ResourceConfigOption {
		fn into_array(self) -> [ResourceConfigOption; 1] {
			[self]
		}
	}
}

pub(super) use private::ResourceConfigOption;

// ----------

pub fn drop_on_unmatching_slash() -> ResourceConfigOption {
	ResourceConfigOption::DropOnUnmatchingSlash
}

pub fn handle_on_unmatching_slash() -> ResourceConfigOption {
	ResourceConfigOption::HandleOnUnmatchingSlash
}

pub fn subtree_handler() -> ResourceConfigOption {
	ResourceConfigOption::SubtreeHandler
}

pub fn modify_request_extensions<Func>(modifier: Func) -> ResourceConfigOption
where
	Func: Fn(&mut Extensions) + Clone + Send + Sync + 'static,
{
	let request_extensions_modifier_layer = RequestExtensionsModifierLayer::new(modifier);

	ResourceConfigOption::ModifyRequestExtensions(request_extensions_modifier_layer)
}
