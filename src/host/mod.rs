use core::panic;
use std::str::FromStr;

use http::Uri;

use crate::{
	common::IntoArray,
	handler::HandlerKind,
	pattern::{Pattern, Similarity},
	resource::Resource,
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

		Self::with_pattern(host_pattern, root)
	}

	pub(crate) fn with_pattern(host_pattern: Pattern, mut root: Resource) -> Self {
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
	pub(crate) fn pattern_string(&self) -> String {
		self.pattern.to_string()
	}

	#[inline(always)]
	pub(crate) fn compare_pattern(&self, other_host_pattern: &Pattern) -> Similarity {
		self.pattern.compare(other_host_pattern)
	}

	#[inline(always)]
	pub(crate) fn root_mut(&mut self) -> &mut Resource {
		&mut self.root_resource
	}

	#[inline(always)]
	pub(crate) fn root(&mut self) -> Resource {
		std::mem::replace(
			&mut self.root_resource,
			Resource::with_pattern(Pattern::default()),
		)
	}

	#[inline(always)]
	pub(crate) fn set_root(&mut self, root: Resource) {
		if self.root_resource.pattern_string() != "" {
			panic!("host already has a root resource");
		}

		self.root_resource = root;
	}

	pub(crate) fn merge_or_replace_root(&mut self, mut new_root: Resource) {
		if !new_root.has_some_effect() {
			self.root_resource.keep_subresources(new_root);
		} else if !self.root_resource.has_some_effect() {
			new_root.keep_subresources(self.root());
			self.root_resource = new_root;
		} else {
			panic!(
				"conflicting root resources for a host '{}'",
				self.pattern_string()
			)
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
