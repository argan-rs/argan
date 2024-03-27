use crate::{
	common::SCOPE_VALIDITY,
	handler::HandlerKind,
	pattern::{split_uri_host_and_path, Pattern, Similarity},
	resource::{self, Resource},
};

// --------------------------------------------------

mod service;

pub use service::HostService;

use self::service::{ArcHostService, LeakedHostService};

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
		let host_pattern = match parse_host_pattern(host_pattern) {
			Ok(host_pattern) => host_pattern,
			Err(HostPatternError::Empty) => panic!("empty host pattern"),
			Err(HostPatternError::Wildcard) => panic!("host pattern cannot be a wildcard"),
		};

		if !root.is("/") {
			panic!("host can only have a root resource");
		}

		Self::with_pattern(host_pattern, root)
	}

	pub(crate) fn with_pattern(host_pattern: Pattern, mut root: Resource) -> Self {
		if root.host_pattern_ref().is_none() {
			root.set_host_pattern(host_pattern.clone());
		} else {
			let resource_host_pattern = root.host_pattern_ref().expect(SCOPE_VALIDITY);

			if resource_host_pattern.compare(&host_pattern) != Similarity::Same {
				panic!(
					"resource is intended to belong to a host {}",
					resource_host_pattern.to_string(),
				);
			}
		}

		Self {
			pattern: host_pattern,
			root_resource: root,
		}
	}

	// -------------------------

	#[inline(always)]
	pub fn is<P: AsRef<str>>(&self, pattern: P) -> bool {
		let pattern = Pattern::parse(pattern.as_ref());

		self.pattern.compare(&pattern) == Similarity::Same
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

	#[inline(always)]
	pub fn into_arc_service(self) -> ArcHostService {
		ArcHostService::from(self.into_service())
	}

	#[inline(always)]
	pub fn into_leaked_service(self) -> LeakedHostService {
		LeakedHostService::from(self.into_service())
	}
}

pub(crate) fn parse_host_pattern<P: AsRef<str>>(
	host_pattern: P,
) -> Result<Pattern, HostPatternError> {
	let host_pattern_str = host_pattern.as_ref();

	if host_pattern_str.is_empty() {
		return Err(HostPatternError::Empty);
	}

	let host_pattern_str = host_pattern_str
		.strip_prefix("https://")
		.or_else(|| host_pattern_str.strip_prefix("http://"))
		.unwrap_or(host_pattern_str);

	let host_pattern = if host_pattern_str.ends_with('/') {
		Pattern::parse(&host_pattern_str[..host_pattern_str.len() - 1])
	} else {
		Pattern::parse(host_pattern_str)
	};

	if host_pattern.is_wildcard() {
		return Err(HostPatternError::Wildcard);
	}

	Ok(host_pattern)
}

#[derive(Debug, crate::ImplError)]
pub(crate) enum HostPatternError {
	#[error("empty host pattern")]
	Empty,
	#[error("wildcard host pattern")]
	Wildcard,
}

// --------------------------------------------------------------------------------
