use serde::{
	de::{DeserializeSeed, EnumAccess, MapAccess, SeqAccess, VariantAccess, Visitor},
	Deserializer,
};

use crate::pattern::{Param, Params};

use super::{from_param::FromParam, FromStr, E};

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

				FromStr::new(some_value).$deserialize(visitor)
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
		visitor.visit_seq(FromPathParamsSeqAccess::new(self))
	}

	fn deserialize_tuple<V: Visitor<'de>>(
		self,
		_len: usize,
		visitor: V,
	) -> Result<V::Value, Self::Error> {
		visitor.visit_seq(FromPathParamsSeqAccess::new(self))
	}

	fn deserialize_tuple_struct<V: Visitor<'de>>(
		self,
		_name: &'static str,
		_len: usize,
		visitor: V,
	) -> Result<V::Value, Self::Error> {
		visitor.visit_seq(FromPathParamsSeqAccess::new(self))
	}

	fn deserialize_map<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
		visitor.visit_map(FromPathParamsMapAccess::new(self))
	}

	fn deserialize_struct<V: Visitor<'de>>(
		self,
		name: &'static str,
		fields: &'static [&'static str],
		visitor: V,
	) -> Result<V::Value, Self::Error> {
		visitor.visit_map(FromPathParamsMapAccess::new(self))
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

// -------------------------

struct FromPathParamsSeqAccess<'p, 'de>(&'p mut FromPathParams<'de>);

impl<'p, 'de> FromPathParamsSeqAccess<'p, 'de> {
	#[inline]
	fn new(from_path_params: &'p mut FromPathParams<'de>) -> Self {
		Self(from_path_params)
	}
}

impl<'de> SeqAccess<'de> for FromPathParamsSeqAccess<'_, 'de> {
	type Error = E;

	fn next_element_seed<T: DeserializeSeed<'de>>(
		&mut self,
		seed: T,
	) -> Result<Option<T::Value>, Self::Error> {
		let Some(param) = self.0.next() else {
			return Ok(None);
		};

		seed.deserialize(FromParam::new(&param)).map(Some)
	}
}

// -------------------------

struct FromPathParamsMapAccess<'p, 'de>(&'p mut FromPathParams<'de>, Option<&'de str>);

impl<'p, 'de> FromPathParamsMapAccess<'p, 'de> {
	#[inline]
	fn new(from_path_params: &'p mut FromPathParams<'de>) -> Self {
		Self(from_path_params, None)
	}
}

impl<'de> MapAccess<'de> for FromPathParamsMapAccess<'_, 'de> {
	type Error = E;

	fn next_key_seed<K: DeserializeSeed<'de>>(
		&mut self,
		seed: K,
	) -> Result<Option<K::Value>, Self::Error> {
		let Some(param) = self.0.next() else {
			return Ok(None);
		};

		self.1 = param.value();

		seed.deserialize(FromStr::new(Some(param.name()))).map(Some)
	}

	fn next_value_seed<V: DeserializeSeed<'de>>(&mut self, seed: V) -> Result<V::Value, Self::Error> {
		seed.deserialize(FromStr::new(self.1.take()))
	}
}

// -------------------------

struct FromPathParamsEnumAccess<'p, 'de>(&'p mut FromPathParams<'de>);

impl<'p, 'de> FromPathParamsEnumAccess<'p, 'de> {
	#[inline]
	fn new(from_path_params: &'p mut FromPathParams<'de>) -> Self {
		Self(from_path_params)
	}
}

impl<'de> EnumAccess<'de> for FromPathParamsEnumAccess<'_, 'de> {
	type Error = E;
	type Variant = Self;

	fn variant_seed<V: DeserializeSeed<'de>>(
		self,
		seed: V,
	) -> Result<(V::Value, Self::Variant), Self::Error> {
		let value = seed.deserialize(self.0.by_ref())?;

		Ok((value, self))
	}
}

impl<'de> VariantAccess<'de> for FromPathParamsEnumAccess<'_, 'de> {
	type Error = E;

	fn unit_variant(self) -> Result<(), Self::Error> {
		Ok(())
	}

	fn newtype_variant_seed<T: DeserializeSeed<'de>>(self, seed: T) -> Result<T::Value, Self::Error> {
		seed.deserialize(self.0)
	}

	fn tuple_variant<V: Visitor<'de>>(
		self,
		_len: usize,
		visitor: V,
	) -> Result<V::Value, Self::Error> {
		self.0.deserialize_seq(visitor)
	}

	fn struct_variant<V: Visitor<'de>>(
		self,
		fields: &'static [&'static str],
		visitor: V,
	) -> Result<V::Value, Self::Error> {
		self.0.deserialize_map(visitor)
	}
}

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------
