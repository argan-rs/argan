use super::utils::*;

// --------------------------------------------------

pub struct Resource {
	name: &'static str,

	static_resources: Option<Vec<Resource>>,
	pattern_resources: Option<Vec<Resource>>,
	wildcard_resource: Option<Box<Resource>>,

	request_receiver: Option<BoxedService>,
	request_passer: Option<BoxedService>,
	request_handler: Option<BoxedService>,

	// TODO: configs, state, redirect, parent

	is_subtree_handler: bool
}

