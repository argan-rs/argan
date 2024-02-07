use std::str::FromStr;

use http::Uri;

use crate::{
	common::IntoArray,
	handler::HandlerKind,
	middleware::LayerTarget,
	pattern::Pattern,
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
	pub fn new<P>(host_pattern: P) -> Self
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

		Self {
			pattern: host_pattern.clone(),
			root_resource: Resource::with_uri_patterns(
				Some(host_pattern),
				Vec::new(),
				Pattern::parse("/"),
				false,
			),
		}
	}

	#[inline(always)]
	pub fn add_resource<R, const N: usize>(&mut self, new_resources: R)
	where
		R: IntoArray<Resource, N>,
	{
		self.root_resource.add_subresource(new_resources);
	}

	#[inline(always)]
	pub fn add_resource_under<P, R, const N: usize>(&mut self, relative_path: P, new_resources: R)
	where
		P: AsRef<str>,
		R: IntoArray<Resource, N>,
	{
		self
			.root_resource
			.add_subresource_under(relative_path, new_resources)
	}

	#[inline(always)]
	pub fn subresource_mut<P>(&mut self, relative_path: P) -> &mut Resource
	where
		P: AsRef<str>,
	{
		self.root_resource.subresource_mut(relative_path)
	}

	#[inline(always)]
	pub fn add_extension<E: Clone + Send + Sync + 'static>(&mut self, extension: E) {
		self.root_resource.add_extension(extension)
	}

	#[inline(always)]
	pub fn set_handler<H, const N: usize>(&mut self, handler_kinds: H)
	where
		H: IntoArray<HandlerKind, N>,
	{
		self.root_resource.set_handler(handler_kinds)
	}

	#[inline(always)]
	pub fn add_layer<L, const N: usize>(&mut self, layer_targets: L)
	where
		L: IntoArray<LayerTarget, N>,
	{
		self.root_resource.add_layer(layer_targets)
	}

	#[inline(always)]
	pub fn set_config<C, const N: usize>(&mut self, config_options: C)
	where
		C: IntoArray<ConfigOption, N>,
	{
		self.root_resource.set_config(config_options)
	}

	#[inline(always)]
	pub fn for_each_subresource<T, F>(&mut self, mut param: T, mut func: F) -> T
	where
		F: FnMut(&mut T, &mut Resource) -> Iteration,
	{
		self.root_resource.for_each_subresource(param, func)
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
