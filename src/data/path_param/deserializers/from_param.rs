use serde::{
	de::{DeserializeSeed, EnumAccess, MapAccess, SeqAccess, Visitor},
	forward_to_deserialize_any, Deserializer,
};

use crate::pattern::Param;

use super::{FromStr, E};

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

pub(super) struct FromParam<'de> {
	some_name: Option<&'de str>,
	some_option_value: Option<Option<&'de str>>, // To support deserialize_tuple and SeqAccess we need double Option.
}

impl<'de> FromParam<'de> {
	#[inline]
	pub(super) fn new(param: &'de Param<'de>) -> Self {
		Self {
			some_name: Some(param.name()),
			some_option_value: Some(param.value()),
		}
	}
}

// --------------------------------------------------

macro_rules! declare_deserialize_for_simple_types {
	($($deserialize:ident)*) => {
		$(
			fn $deserialize<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
				let some_value = self.some_option_value.ok_or(E)?;

				FromStr(some_value).$deserialize(visitor)
			}
		)*
	};
}

impl<'de> Deserializer<'de> for FromParam<'de> {
	type Error = E;

	declare_deserialize_for_simple_types!(
		deserialize_any
		deserialize_ignored_any
		deserialize_bool
		deserialize_i8
		deserialize_i16
		deserialize_i32
		deserialize_i64
		deserialize_u8
		deserialize_u16
		deserialize_u32
		deserialize_u64
		deserialize_f32
		deserialize_f64
		deserialize_char
		deserialize_str
		deserialize_string
		deserialize_bytes
		deserialize_byte_buf
		deserialize_option
		deserialize_unit
		deserialize_identifier
	);

	fn deserialize_unit_struct<V: Visitor<'de>>(
		self,
		_name: &'static str,
		visitor: V,
	) -> Result<V::Value, Self::Error> {
		visitor.visit_unit()
	}

	fn deserialize_newtype_struct<V: Visitor<'de>>(
		self,
		_name: &'static str,
		visitor: V,
	) -> Result<V::Value, Self::Error> {
		visitor.visit_newtype_struct(self)
	}

	fn deserialize_tuple<V: Visitor<'de>>(
		self,
		len: usize,
		visitor: V,
	) -> Result<V::Value, Self::Error> {
		if len < 3 {
			return visitor.visit_seq(self);
		}

		Err(E)
	}

	fn deserialize_map<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
		visitor.visit_map(self)
	}

	fn deserialize_enum<V: Visitor<'de>>(
		self,
		name: &'static str,
		variants: &'static [&'static str],
		visitor: V,
	) -> Result<V::Value, Self::Error> {
		todo!()
	}

	forward_to_deserialize_any! { seq tuple_struct struct }
}

impl<'de> SeqAccess<'de> for FromParam<'de> {
	type Error = E;

	fn next_element_seed<T: DeserializeSeed<'de>>(
		&mut self,
		seed: T,
	) -> Result<Option<T::Value>, Self::Error> {
		if self.some_name.is_some() {
			return seed.deserialize(FromStr(self.some_name.take())).map(Some);
		}

		if let Some(some_value) = self.some_option_value {
			return seed.deserialize(FromStr(some_value)).map(Some);
		}

		Ok(None)
	}
}

impl<'de> MapAccess<'de> for FromParam<'de> {
	type Error = E;

	fn next_key_seed<K: DeserializeSeed<'de>>(
		&mut self,
		seed: K,
	) -> Result<Option<K::Value>, Self::Error> {
		if self.some_name.is_some() {
			return seed.deserialize(FromStr(self.some_name.take())).map(Some);
		}

		Ok(None)
	}

	fn next_value_seed<V: DeserializeSeed<'de>>(&mut self, seed: V) -> Result<V::Value, Self::Error> {
		if let Some(some_value) = self.some_option_value {
			return seed.deserialize(FromStr(some_value));
		}

		Err(E)
	}
}

impl<'de> EnumAccess<'de> for FromParam<'de> {
	type Error = E;
	type Variant = FromStr<'de>;

	fn variant_seed<V: DeserializeSeed<'de>>(
		self,
		seed: V,
	) -> Result<(V::Value, Self::Variant), Self::Error> {
		let some_value = self.some_option_value.ok_or(E)?;
		let mut deserializer = FromStr(some_value);

		seed
			.deserialize(deserializer.clone())
			.map(|value| (value, deserializer))
	}
}

// --------------------------------------------------------------------------------
