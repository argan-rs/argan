//! Host service types.

// ----------

use crate::{
	common::{IntoArray, SCOPE_VALIDITY},
	pattern::{Pattern, Similarity},
	resource::Resource,
};

// --------------------------------------------------

mod service;

pub use service::{ArcHostService, HostService, LeakedHostService};

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

/// Representation of the *host* subcomponent of the URI.
pub struct Host {
	pattern: Pattern,
	root_resource: Resource,
}

impl Host {
	/// Creates a new `Host` with the given pattern and root resource.
	///
	/// ```
	/// use argan::{Host, Resource};
	///
	/// let root = Resource::new("/");
	/// let host = Host::new("http://{sub_domain}.example.com", root);
	/// ```
	///
	/// The `Host` node checks the request's host and, if matches, passes the request to
	/// its root resource.
	pub fn new<P>(host_pattern: P, root: Resource) -> Self
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
					resource_host_pattern,
				);
			}
		}

		Self {
			pattern: host_pattern,
			root_resource: root,
		}
	}

	// -------------------------

	/// Checks whether the given `pattern` is the `Host`'s pattern.
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
	pub(crate) fn pattern_ref(&self) -> &Pattern {
		&self.pattern
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

	pub(crate) fn into_pattern_and_root(self) -> (Pattern, Resource) {
		let Host {
			pattern,
			root_resource,
		} = self;

		(pattern, root_resource)
	}

	/// Converts the `Host` into a service.
	#[inline(always)]
	pub fn into_service(self) -> HostService {
		let Host {
			pattern,
			root_resource,
		} = self;

		HostService::new(pattern, root_resource.into_service())
	}

	/// Converts the `Host` into a service that uses `Arc` internally.
	#[inline(always)]
	pub fn into_arc_service(self) -> ArcHostService {
		ArcHostService::from(self.into_service())
	}

	/// Converts the `Host` into a service with a leaked `&'static`.
	#[inline(always)]
	pub fn into_leaked_service(self) -> LeakedHostService {
		LeakedHostService::from(self.into_service())
	}
}

impl IntoArray<Host, 1> for Host {
	fn into_array(self) -> [Host; 1] {
		[self]
	}
}

// --------------------------------------------------

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

	let host_pattern = host_pattern_str
		.strip_suffix('/')
		.map_or(Pattern::parse(host_pattern_str), Pattern::parse);

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
