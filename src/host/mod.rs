use core::panic;
use std::str::FromStr;

use http::Uri;

use crate::{
	common::IntoArray,
	handler::HandlerKind,
	middleware::LayerTarget,
	pattern::{Pattern, Similarity},
	resource::{config::ConfigOption, Iteration, Resource},
};

// --------------------------------------------------

mod service;

pub use service::HostService;

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

pub struct Host {
	pattern: Pattern,
	root_resource: Resource,
}

impl Host {
	pub fn new<P>(host_pattern: P, mut root: Resource) -> Self
	where
		P: AsRef<str>,
	{
		let uri = Uri::from_str(host_pattern.as_ref()).expect("host pattern should be a valid URI");
		let host = uri
			.host()
			.expect("pattern should have an authority component");

		let host_pattern = Pattern::parse(host);
		if host_pattern.is_wildcard() {
			panic!("host pattern cannot be a wildcard");
		}

		if let Pattern::Regex(names, Some(_)) = &host_pattern {
			panic!("regex host pattern must be complete");
		}

		if root.pattern_string() != "/" {
			panic!("host can only have a root resource");
		}

		if root
			.host_pattern_ref()
			.is_some_and(|resource_host_pattern| {
				if let Pattern::Regex(resource_host_names, None) = resource_host_pattern {
					if let Pattern::Regex(host_names, _) = &host_pattern {
						if resource_host_names.pattern_name() != host_names.pattern_name() {
							panic!(
								"resource is intended to belong to a host {}",
								resource_host_pattern.to_string(),
							);
						}

						return true;
					}
				} else if resource_host_pattern.compare(&host_pattern) != Similarity::Same {
					panic!(
						"resource is intended to belong to a host {}",
						resource_host_pattern.to_string(),
					);
				}

				false
			}) {
			// Root doesn't have the regex part of the host pattern. We need to set it.
			root.set_host_pattern(host_pattern.clone());
		}

		Self {
			pattern: host_pattern,
			root_resource: root,
		}
	}

	#[inline(always)]
	pub fn into_service(self) -> HostService {
		let Host {
			pattern,
			root_resource,
		} = self;

		HostService::new(pattern, root_resource.into_service())
	}
}

// --------------------------------------------------------------------------------
