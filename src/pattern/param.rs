use std::{error::Error, fmt::Display};

use regex::{CaptureNames, Captures};
use serde::de::{
	DeserializeSeed, Deserializer, EnumAccess, IntoDeserializer, MapAccess, SeqAccess, VariantAccess,
	Visitor,
};

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

#[derive(Debug)]
pub(crate) struct E;

impl Display for E {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "Deserializer Error")
	}
}

impl Error for E {}

impl serde::de::Error for E {
	fn custom<T>(msg: T) -> Self
	where
		T: Display,
	{
		E
	}
}

// --------------------------------------------------

#[derive(Debug)]
pub(crate) enum Params<'p> {
	Regex(&'p str, CaptureNames<'p>, Captures<'p>),
	Wildcard(&'p str, Option<&'p str>),
}

impl<'p> Params<'p> {
	pub(crate) fn name(&self) -> &'p str {
		match self {
			Self::Regex(name, _, _) => name,
			Self::Wildcard(name, _) => name,
		}
	}

	fn value(&self, name: &str) -> Option<&'p str> {
		match self {
			Self::Regex(_, _, captures) => captures.name(name).map(|match_value| match_value.as_str()),
			Self::Wildcard(wildcard_name, value) => {
				if name == *wildcard_name {
					*value
				} else {
					None
				}
			}
		}
	}
}

impl<'p> Iterator for Params<'p> {
	type Item = Param<'p>;

	fn next(&mut self) -> Option<Self::Item> {
		match self {
			Self::Regex(_, ref mut capture_names, captures) => {
				for some_name in capture_names.by_ref() {
					let Some(name) = some_name else { continue };

					let some_value = captures.name(name);

					return Some(Param::new(name, some_value.map(|value| value.as_str())));
				}

				None
			}
			Self::Wildcard(name, some_value) => {
				let Some(value) = some_value.take() else {
					return None;
				};

				Some(Param::new(name, Some(value)))
			}
		}
	}
}

impl<'p> Deserializer<'p> for Params<'p> {
	type Error = E;

	fn deserialize_any<V: Visitor<'p>>(self, visitor: V) -> Result<V::Value, Self::Error> {
		todo!()
	}

	fn deserialize_bool<V: Visitor<'p>>(self, visitor: V) -> Result<V::Value, Self::Error> {
		todo!()
	}

	fn deserialize_i8<V: Visitor<'p>>(self, visitor: V) -> Result<V::Value, Self::Error> {
		todo!()
	}

	fn deserialize_i16<V: Visitor<'p>>(self, visitor: V) -> Result<V::Value, Self::Error> {
		todo!()
	}

	fn deserialize_i32<V: Visitor<'p>>(self, visitor: V) -> Result<V::Value, Self::Error> {
		todo!()
	}

	fn deserialize_i64<V: Visitor<'p>>(self, visitor: V) -> Result<V::Value, Self::Error> {
		todo!()
	}

	fn deserialize_u8<V: Visitor<'p>>(self, visitor: V) -> Result<V::Value, Self::Error> {
		todo!()
	}

	fn deserialize_u16<V: Visitor<'p>>(self, visitor: V) -> Result<V::Value, Self::Error> {
		todo!()
	}

	fn deserialize_u32<V: Visitor<'p>>(self, visitor: V) -> Result<V::Value, Self::Error> {
		todo!()
	}

	fn deserialize_u64<V: Visitor<'p>>(self, visitor: V) -> Result<V::Value, Self::Error> {
		todo!()
	}

	fn deserialize_f32<V: Visitor<'p>>(self, visitor: V) -> Result<V::Value, Self::Error> {
		todo!()
	}

	fn deserialize_f64<V: Visitor<'p>>(self, visitor: V) -> Result<V::Value, Self::Error> {
		todo!()
	}

	fn deserialize_char<V: Visitor<'p>>(self, visitor: V) -> Result<V::Value, Self::Error> {
		todo!()
	}

	fn deserialize_str<V: Visitor<'p>>(self, visitor: V) -> Result<V::Value, Self::Error> {
		todo!()
	}

	fn deserialize_string<V: Visitor<'p>>(self, visitor: V) -> Result<V::Value, Self::Error> {
		todo!()
	}

	fn deserialize_bytes<V: Visitor<'p>>(self, visitor: V) -> Result<V::Value, Self::Error> {
		todo!()
	}

	fn deserialize_byte_buf<V: Visitor<'p>>(self, visitor: V) -> Result<V::Value, Self::Error> {
		todo!()
	}

	fn deserialize_option<V: Visitor<'p>>(self, visitor: V) -> Result<V::Value, Self::Error> {
		todo!()
	}

	fn deserialize_unit<V: Visitor<'p>>(self, visitor: V) -> Result<V::Value, Self::Error> {
		todo!()
	}

	fn deserialize_unit_struct<V: Visitor<'p>>(
		self,
		_name: &'static str,
		visitor: V,
	) -> Result<V::Value, Self::Error> {
		todo!()
	}

	fn deserialize_newtype_struct<V: Visitor<'p>>(
		self,
		_name: &'static str,
		visitor: V,
	) -> Result<V::Value, Self::Error> {
		todo!()
	}

	fn deserialize_seq<V: Visitor<'p>>(self, visitor: V) -> Result<V::Value, Self::Error> {
		todo!()
	}

	fn deserialize_tuple<V: Visitor<'p>>(
		self,
		len: usize,
		visitor: V,
	) -> Result<V::Value, Self::Error> {
		todo!()
	}

	fn deserialize_tuple_struct<V: Visitor<'p>>(
		self,
		_name: &'static str,
		len: usize,
		visitor: V,
	) -> Result<V::Value, Self::Error> {
		todo!()
	}

	fn deserialize_map<V: Visitor<'p>>(self, visitor: V) -> Result<V::Value, Self::Error> {
		todo!()
	}

	fn deserialize_struct<V: Visitor<'p>>(
		self,
		name: &'static str,
		fields: &'static [&'static str],
		visitor: V,
	) -> Result<V::Value, Self::Error> {
		todo!()
	}

	fn deserialize_enum<V: Visitor<'p>>(
		self,
		name: &'static str,
		variants: &'static [&'static str],
		visitor: V,
	) -> Result<V::Value, Self::Error> {
		todo!()
	}

	fn deserialize_identifier<V: Visitor<'p>>(self, visitor: V) -> Result<V::Value, Self::Error> {
		todo!()
	}

	fn deserialize_ignored_any<V: Visitor<'p>>(self, visitor: V) -> Result<V::Value, Self::Error> {
		todo!()
	}
}

// --------------------------------------------------

pub(crate) struct Param<'p> {
	name: &'p str,
	some_value: Option<&'p str>,
}

impl<'p> Param<'p> {
	#[inline]
	fn new(name: &'p str, some_value: Option<&'p str>) -> Self {
		Self { name, some_value }
	}

	#[inline]
	pub(crate) fn name(&self) -> &'p str {
		self.name
	}

	pub(crate) fn value(&self) -> Option<&'p str> {
		self.some_value
	}
}

macro_rules! declare_deserialize_for_parsable {
	($deserialize:ident, $visit:ident, $type:ty) => {
		fn $deserialize<V: Visitor<'p>>(self, visitor: V) -> Result<V::Value, Self::Error> {
			let value = self.some_value.ok_or(E)?;

			match value.parse() {
				Ok(value) => visitor.$visit(value),
				Err(_) => Err(E),
			}
		}
	};
}

impl<'p> Deserializer<'p> for &mut Param<'p> {
	type Error = E;

	fn deserialize_any<V: Visitor<'p>>(self, visitor: V) -> Result<V::Value, Self::Error> {
		let value = self.some_value.ok_or(E)?;

		visitor.visit_borrowed_str(value)
	}

	declare_deserialize_for_parsable!(deserialize_bool, visit_bool, bool);
	declare_deserialize_for_parsable!(deserialize_i8, visit_i8, i8);
	declare_deserialize_for_parsable!(deserialize_i16, visit_i16, i16);
	declare_deserialize_for_parsable!(deserialize_i32, visit_i32, i32);
	declare_deserialize_for_parsable!(deserialize_i64, visit_i64, i64);
	declare_deserialize_for_parsable!(deserialize_u8, visit_u8, u8);
	declare_deserialize_for_parsable!(deserialize_u16, visit_u16, u16);
	declare_deserialize_for_parsable!(deserialize_u32, visit_u32, u32);
	declare_deserialize_for_parsable!(deserialize_u64, visit_u64, u64);
	declare_deserialize_for_parsable!(deserialize_f32, visit_f32, f32);
	declare_deserialize_for_parsable!(deserialize_f64, visit_f64, f64);

	fn deserialize_char<V: Visitor<'p>>(self, visitor: V) -> Result<V::Value, Self::Error> {
		let value = self.some_value.ok_or(E)?;
		let mut chars = value.chars();
		let value = chars.next().ok_or(E)?;

		if chars.any(|remaining| true) {
			return Err(E);
		}

		visitor.visit_char(value)
	}

	fn deserialize_str<V: Visitor<'p>>(self, visitor: V) -> Result<V::Value, Self::Error> {
		let value = self.some_value.ok_or(E)?;

		visitor.visit_borrowed_str(value)
	}

	fn deserialize_string<V: Visitor<'p>>(self, visitor: V) -> Result<V::Value, Self::Error> {
		let value = self.some_value.ok_or(E)?;

		visitor.visit_string(value.to_owned())
	}

	fn deserialize_bytes<V: Visitor<'p>>(self, visitor: V) -> Result<V::Value, Self::Error> {
		let value = self.some_value.ok_or(E)?;

		visitor.visit_borrowed_bytes(value.as_bytes())
	}

	fn deserialize_byte_buf<V: Visitor<'p>>(self, visitor: V) -> Result<V::Value, Self::Error> {
		let value = self.some_value.ok_or(E)?;

		visitor.visit_byte_buf(value.as_bytes().to_owned())
	}

	fn deserialize_option<V: Visitor<'p>>(self, visitor: V) -> Result<V::Value, Self::Error> {
		if self.some_value.is_some() {
			visitor.visit_some(self)
		} else {
			visitor.visit_none()
		}
	}

	fn deserialize_unit<V: Visitor<'p>>(self, visitor: V) -> Result<V::Value, Self::Error> {
		visitor.visit_unit()
	}

	fn deserialize_unit_struct<V: Visitor<'p>>(
		self,
		_name: &'static str,
		visitor: V,
	) -> Result<V::Value, Self::Error> {
		visitor.visit_unit()
	}

	fn deserialize_newtype_struct<V: Visitor<'p>>(
		self,
		_name: &'static str,
		visitor: V,
	) -> Result<V::Value, Self::Error> {
		visitor.visit_newtype_struct(self)
	}

	fn deserialize_seq<V: Visitor<'p>>(self, visitor: V) -> Result<V::Value, Self::Error> {
		visitor.visit_seq(NameValue::from(self))
	}

	fn deserialize_tuple<V: Visitor<'p>>(
		self,
		_len: usize,
		visitor: V,
	) -> Result<V::Value, Self::Error> {
		self.deserialize_seq(visitor)
	}

	fn deserialize_tuple_struct<V: Visitor<'p>>(
		self,
		_name: &'static str,
		len: usize,
		visitor: V,
	) -> Result<V::Value, Self::Error> {
		self.deserialize_seq(visitor)
	}

	fn deserialize_map<V: Visitor<'p>>(self, visitor: V) -> Result<V::Value, Self::Error> {
		visitor.visit_map(NameValue::from(self))
	}

	fn deserialize_struct<V: Visitor<'p>>(
		self,
		name: &'static str,
		fields: &'static [&'static str],
		visitor: V,
	) -> Result<V::Value, Self::Error> {
		self.deserialize_map(visitor)
	}

	fn deserialize_enum<V: Visitor<'p>>(
		self,
		name: &'static str,
		variants: &'static [&'static str],
		visitor: V,
	) -> Result<V::Value, Self::Error> {
		visitor.visit_enum(self)
	}

	fn deserialize_identifier<V: Visitor<'p>>(self, visitor: V) -> Result<V::Value, Self::Error> {
		visitor.visit_str(self.name)
	}

	fn deserialize_ignored_any<V: Visitor<'p>>(self, visitor: V) -> Result<V::Value, Self::Error> {
		self.deserialize_unit(visitor)
	}
}

pub(crate) struct NameValue<'p>(Option<&'p str>, Option<&'p str>);

impl<'p> From<&mut Param<'p>> for NameValue<'p> {
	fn from(param: &mut Param<'p>) -> Self {
		Self(
			Some(param.name),
			param.some_value, /* param.some_value (Option<&'p str>) is a Copy type */
		)
	}
}

impl<'p> SeqAccess<'p> for NameValue<'p> {
	type Error = E;

	fn next_element_seed<T: DeserializeSeed<'p>>(
		&mut self,
		seed: T,
	) -> Result<Option<T::Value>, Self::Error> {
		if let Some(name) = self.0.take() {
			return seed.deserialize(name.into_deserializer()).map(Some);
		}

		if let Some(value) = self.1.take() {
			return seed.deserialize(value.into_deserializer()).map(Some);
		}

		Ok(None)
	}
}

impl<'p> MapAccess<'p> for NameValue<'p> {
	type Error = E;

	fn next_key_seed<K: DeserializeSeed<'p>>(
		&mut self,
		seed: K,
	) -> Result<Option<K::Value>, Self::Error> {
		if let Some(name) = self.0.take() {
			return seed.deserialize(name.into_deserializer()).map(Some);
		}

		Ok(None)
	}

	fn next_value_seed<V: DeserializeSeed<'p>>(&mut self, seed: V) -> Result<V::Value, Self::Error> {
		if let Some(value) = self.1.take() {
			return seed.deserialize(value.into_deserializer());
		}

		Err(E)
	}
}

impl<'p> EnumAccess<'p> for &mut Param<'p> {
	type Error = E;
	type Variant = Self;

	fn variant_seed<V: DeserializeSeed<'p>>(
		self,
		seed: V,
	) -> Result<(V::Value, Self::Variant), Self::Error> {
		seed.deserialize(&mut *self).map(|value| (value, self))
	}
}

impl<'p> VariantAccess<'p> for &mut Param<'p> {
	type Error = E;

	fn unit_variant(self) -> Result<(), Self::Error> {
		Ok(())
	}

	fn newtype_variant_seed<T: DeserializeSeed<'p>>(self, seed: T) -> Result<T::Value, Self::Error> {
		seed.deserialize(self)
	}

	fn tuple_variant<V: Visitor<'p>>(self, _len: usize, visitor: V) -> Result<V::Value, Self::Error> {
		visitor.visit_seq(NameValue::from(self))
	}

	fn struct_variant<V: Visitor<'p>>(
		self,
		fields: &'static [&'static str],
		visitor: V,
	) -> Result<V::Value, Self::Error> {
		visitor.visit_map(NameValue::from(self))
	}
}
