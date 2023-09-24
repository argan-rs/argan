use serde::{
	de::{DeserializeSeed, MapAccess, SeqAccess, Visitor},
	Deserializer,
};

use crate::pattern::{Param, Params};

use super::{FromStr, E};

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

pub(crate) struct FromPathParams<'de> {
	params: &'de mut Vec<Params<'de>>,
	next_segment_params_index: usize,
}

impl<'de> FromPathParams<'de> {
	#[inline]
	pub(crate) fn new(params: &'de mut Vec<Params<'de>>) -> Self {
		FromPathParams {
			params,
			next_segment_params_index: 0,
		}
	}
}

impl<'de> Iterator for FromPathParams<'de> {
	type Item = Param<'de>;

	fn next(&mut self) -> Option<Self::Item> {
		while self.next_segment_params_index < self.params.len() {
			let params = &mut self.params[self.next_segment_params_index];
			let param = params.next();

			if param.is_some() {
				return param;
			}

			self.next_segment_params_index += 1;
		}

		None
	}
}

// --------------------------------------------------

macro_rules! declare_deserialize_for_simple_types {
	($($deserialize:ident)*) => {
		$(
			fn $deserialize<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
				let some_value = self.next().and_then(|param| param.value());

				FromStr(some_value).$deserialize(visitor)
			}
		)*
	};
}

impl<'de> Deserializer<'de> for &mut FromPathParams<'de> {
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

	fn deserialize_seq<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
		visitor.visit_seq(self)
	}

	fn deserialize_tuple<V: Visitor<'de>>(
		self,
		_len: usize,
		visitor: V,
	) -> Result<V::Value, Self::Error> {
		visitor.visit_seq(self)
	}

	fn deserialize_tuple_struct<V: Visitor<'de>>(
		self,
		_name: &'static str,
		_len: usize,
		visitor: V,
	) -> Result<V::Value, Self::Error> {
		visitor.visit_seq(self)
	}

	fn deserialize_map<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
		todo!()
	}

	fn deserialize_struct<V: Visitor<'de>>(
		self,
		name: &'static str,
		fields: &'static [&'static str],
		visitor: V,
	) -> Result<V::Value, Self::Error> {
		todo!()
	}

	fn deserialize_enum<V: Visitor<'de>>(
		self,
		name: &'static str,
		variants: &'static [&'static str],
		visitor: V,
	) -> Result<V::Value, Self::Error> {
		todo!()
	}
}

impl<'de> SeqAccess<'de> for FromPathParams<'de> {
	type Error = E;

	fn next_element_seed<T: DeserializeSeed<'de>>(
		&mut self,
		seed: T,
	) -> Result<Option<T::Value>, Self::Error> {
		// seed.deserialize(deserializer)

		todo!()
	}
}

impl<'de> MapAccess<'de> for FromPathParams<'de> {
	type Error = E;

	fn next_key_seed<K: DeserializeSeed<'de>>(
		&mut self,
		seed: K,
	) -> Result<Option<K::Value>, Self::Error> {
		// seed.deserialize(deserializer)

		todo!()
	}

	fn next_value_seed<V: DeserializeSeed<'de>>(&mut self, seed: V) -> Result<V::Value, Self::Error> {
		// seed.deserialize(deserializer)

		todo!()
	}
}

// --------------------------------------------------------------------------------
