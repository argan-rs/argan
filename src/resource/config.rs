use crate::{common::SCOPE_VALIDITY, middleware::RequestExtensionsModifierLayer};

use super::*;

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

bit_flags! {
	#[derive(Debug, Clone, PartialEq)]
	pub(super) ConfigFlags: u8 {
		pub(super) NONE = 0b0000;
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
// path_config

// /some_resource !     - send 404 when there is a trailing slash
// /some_resource/ !    - send 404 when there is no trailing slash
// /some_resource !*    - send 404 when there is a tralling slash
//                      - handle requests to non-existent subtree resources
// /some_resource/ !*   - send 404 when there is no tralling slash
//                      - handle requests to non-existent subtree resources
// /some_resource ?     - handle requests even when there is a trailing slash
// /some_resource/ ?    - handle requests even when there no trailing slash
// /some_resource ?*    - handle requests even when there is a trailing slash
//                      - handle requests to non-existent subtree resources
// /some_resource/ ?*   - handle requests even when there is no trailing slash
//                      - handle requests to non-existent subtree resources
// /some_resource *     - handle requests to non-existent subtree resources
// /some_resource/ *    - handle requests to non-existent subtree resources
//
pub(super) fn resource_config_from(path: &str) -> (ConfigFlags, &str) {
	if let Some(config_symbols_position) = path
		.chars()
		.rev()
		.position(|ch| ch == ' ')
		.map(|position| path.len() - position)
	{
		let config_symbols = &path[config_symbols_position..path.len()];
		let mut config_flags = ConfigFlags::default();

		if config_symbols.is_empty() || config_symbols.len() > 2 {
			return (config_flags, path);
		}

		let path = &path[..config_symbols_position - 1];

		let path_is_root = if path != "/" {
			if path.as_bytes().last().unwrap() == &b'/' {
				config_flags.add(ConfigFlags::ENDS_WITH_SLASH);
			}

			false
		} else {
			config_flags.remove(ConfigFlags::REDIRECTS_ON_UNMATCHING_SLASH);

			true
		};

		for symbol in config_symbols.chars() {
			macro_rules! fail {
				() => {
					panic!("invalid config symbols [{}]", config_symbols);
				};
			}

			if symbol == '*' {
				if config_flags.has(ConfigFlags::SUBTREE_HANDLER) {
					fail!();
				}

				config_flags.add(ConfigFlags::SUBTREE_HANDLER);

				continue;
			}

			if config_flags.has(ConfigFlags::REDIRECTS_ON_UNMATCHING_SLASH) {
				config_flags.remove(ConfigFlags::REDIRECTS_ON_UNMATCHING_SLASH);
			} else {
				fail!();
			}

			if symbol == '!' {
				if !path_is_root {
					config_flags.add(ConfigFlags::DROPS_ON_UNMATCHING_SLASH);
				}

				continue;
			}

			if symbol != '?' {
				fail!();
			}
		}

		return (config_flags, path);
	}

	let mut config_flags = ConfigFlags::default();

	if path != "/" {
		if path.as_bytes().last().expect(SCOPE_VALIDITY) == &b'/' {
			config_flags.add(ConfigFlags::ENDS_WITH_SLASH);
		}
	} else {
		config_flags.remove(ConfigFlags::REDIRECTS_ON_UNMATCHING_SLASH);
	}

	(config_flags, path)
}

// --------------------------------------------------
// ResourceConfigOption

config_option! {
	ResourceConfigOption {
		RequestExtensionsModifier(RequestExtensionsModifierLayer),
	}
}

// ----------

pub fn _with_request_extensions_modifier<Func>(modifier: Func) -> ResourceConfigOption
where
	Func: Fn(&mut Extensions) + Clone + Send + Sync + 'static,
{
	let request_extensions_modifier_layer = RequestExtensionsModifierLayer::new(modifier);

	ResourceConfigOption::RequestExtensionsModifier(request_extensions_modifier_layer)
}

// --------------------------------------------------------------------------------

#[cfg(test)]
mod test {
	use super::*;

	#[test]
	fn resource_config_from() {
		let cases = [
			("/", ConfigFlags::NONE, "/"),
			("/ *", ConfigFlags::SUBTREE_HANDLER, "/"),
			("/ ", ConfigFlags::REDIRECTS_ON_UNMATCHING_SLASH, "/ "),
			("/ !?*", ConfigFlags::REDIRECTS_ON_UNMATCHING_SLASH, "/ !?*"),
			(
				"/some_resource",
				ConfigFlags::REDIRECTS_ON_UNMATCHING_SLASH,
				"/some_resource",
			),
			(
				"/some_resource/",
				ConfigFlags::ENDS_WITH_SLASH | ConfigFlags::REDIRECTS_ON_UNMATCHING_SLASH,
				"/some_resource/",
			),
			(
				"/some_resource !",
				ConfigFlags::DROPS_ON_UNMATCHING_SLASH,
				"/some_resource",
			),
			(
				"/some_resource/ !",
				ConfigFlags::ENDS_WITH_SLASH | ConfigFlags::DROPS_ON_UNMATCHING_SLASH,
				"/some_resource/",
			),
			("/some_resource ?", ConfigFlags(0), "/some_resource"),
			(
				"/some_resource/ ?",
				ConfigFlags::ENDS_WITH_SLASH,
				"/some_resource/",
			),
			(
				"/some_resource *",
				ConfigFlags::REDIRECTS_ON_UNMATCHING_SLASH | ConfigFlags::SUBTREE_HANDLER,
				"/some_resource",
			),
			(
				"/some_resource/ *",
				ConfigFlags::ENDS_WITH_SLASH
					| ConfigFlags::REDIRECTS_ON_UNMATCHING_SLASH
					| ConfigFlags::SUBTREE_HANDLER,
				"/some_resource/",
			),
			(
				"/some_resource !*",
				ConfigFlags::DROPS_ON_UNMATCHING_SLASH | ConfigFlags::SUBTREE_HANDLER,
				"/some_resource",
			),
			(
				"/some_resource *!",
				ConfigFlags::DROPS_ON_UNMATCHING_SLASH | ConfigFlags::SUBTREE_HANDLER,
				"/some_resource",
			),
			(
				"/some_resource/ !*",
				ConfigFlags::ENDS_WITH_SLASH
					| ConfigFlags::DROPS_ON_UNMATCHING_SLASH
					| ConfigFlags::SUBTREE_HANDLER,
				"/some_resource/",
			),
			(
				"/some_resource/ *!",
				ConfigFlags::ENDS_WITH_SLASH
					| ConfigFlags::DROPS_ON_UNMATCHING_SLASH
					| ConfigFlags::SUBTREE_HANDLER,
				"/some_resource/",
			),
			(
				"/some_resource ?*",
				ConfigFlags::SUBTREE_HANDLER,
				"/some_resource",
			),
			(
				"/some_resource *?",
				ConfigFlags::SUBTREE_HANDLER,
				"/some_resource",
			),
			(
				"/some_resource/ ?*",
				ConfigFlags::ENDS_WITH_SLASH | ConfigFlags::SUBTREE_HANDLER,
				"/some_resource/",
			),
			(
				"/some_resource/ *?",
				ConfigFlags::ENDS_WITH_SLASH | ConfigFlags::SUBTREE_HANDLER,
				"/some_resource/",
			),
		];

		for case in cases {
			println!("case: {}", case.0);

			let config_flags = super::resource_config_from(case.0);
			assert_eq!(config_flags, (case.1, case.2));
		}
	}

	#[test]
	#[should_panic = "[!]"]
	fn resource_config_from_panic_1() {
		super::resource_config_from("/ !");
	}

	#[test]
	#[should_panic = "[?]"]
	fn resource_config_from_panic_2() {
		super::resource_config_from("/ ?");
	}

	#[test]
	#[should_panic = "[!*]"]
	fn resource_config_from_panic_3() {
		super::resource_config_from("/ !*");
	}

	#[test]
	#[should_panic = "[*!]"]
	fn resource_config_from_panic_4() {
		super::resource_config_from("/ *!");
	}

	#[test]
	#[should_panic = "[?*]"]
	fn resource_config_from_panic_5() {
		super::resource_config_from("/ ?*");
	}

	#[test]
	#[should_panic = "[*?]"]
	fn resource_config_from_panic_6() {
		super::resource_config_from("/ *?");
	}

	#[test]
	#[should_panic = "[!!]"]
	fn resource_config_from_panic_7() {
		super::resource_config_from("/resource !!");
	}

	#[test]
	#[should_panic = "[??]"]
	fn resource_config_from_panic_8() {
		super::resource_config_from("/resource ??");
	}

	#[test]
	#[should_panic = "[!?]"]
	fn resource_config_from_panic_9() {
		super::resource_config_from("/resource !?");
	}

	#[test]
	#[should_panic = "[?!]"]
	fn resource_config_from_panic_10() {
		super::resource_config_from("/resource ?!");
	}
}
