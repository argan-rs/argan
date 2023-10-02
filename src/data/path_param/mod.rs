use std::{
	convert::Infallible,
	future::{ready, Ready},
};

use serde::{de::DeserializeOwned, Deserializer};

// -------------------------

mod deserializers;

use deserializers::FromPath;

use crate::{
	request::{FromRequestHead, Head},
	routing::RoutingState,
};

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

pub struct PathParam<T>(pub T);

impl<'de, T> PathParam<T>
where
	T: DeserializeOwned,
{
	pub fn deserialize<D: Deserializer<'de>>(&mut self, deserializer: D) -> Result<(), D::Error> {
		self.0 = T::deserialize(deserializer)?;

		Ok(())
	}
}

impl<'de, T> FromRequestHead for PathParam<T>
where
	T: DeserializeOwned,
{
	type Error = Infallible;
	type Future = Ready<Result<Self, Self::Error>>;

	fn from_request_head(head: &mut Head) -> Self::Future {
		let mut routing_state = head.extensions.get_mut::<RoutingState>().unwrap();
		let mut from_path = FromPath::new(&mut routing_state.path_params);

		let value = T::deserialize(&mut from_path).unwrap();

		ready(Ok(PathParam(value)))
	}
}

// --------------------------------------------------
