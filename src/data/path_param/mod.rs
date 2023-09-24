use serde::{Deserialize, Deserializer};

// -------------------------

mod deserializers;

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

pub struct PathParam<T>(pub T);

impl<'de, T: Deserialize<'de>> PathParam<T> {
	pub(crate) fn deserialize<D: Deserializer<'de>>(
		&mut self,
		deserializer: D,
	) -> Result<(), D::Error> {
		self.0 = T::deserialize(deserializer)?;

		Ok(())
	}
}

// --------------------------------------------------
