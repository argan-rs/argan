use std::{error::Error, fmt::Display};

use serde::{
	de::{DeserializeSeed, EnumAccess, VariantAccess, Visitor},
	forward_to_deserialize_any, Deserializer,
};

// --------------------------------------------------

mod from_params;
mod from_params_list;

pub(crate) use from_params_list::FromParamsList;

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

#[repr(u8)]
#[derive(Debug, Default, Clone, Copy, PartialEq)]
enum DataType {
	#[default]
	Unknown,
	Single,
	Sequence,
	Tuple,
	Map,
	Struct,
}

// --------------------------------------------------

#[derive(Clone)]
struct FromStr<'de>(Option<&'de str>);

impl<'de> FromStr<'de> {
	#[inline]
	fn new(some_str: Option<&'de str>) -> Self {
		Self(some_str)
	}
}

macro_rules! declare_deserialize_for_parsable {
	($deserialize:ident, $visit:ident, $type:ty) => {
		fn $deserialize<V>(self, visitor: V) -> Result<V::Value, Self::Error>
		where
			V: Visitor<'de>,
		{
			println!("from str: {}", stringify!($deserialize));
			let value = self.0.ok_or(E)?;

			match value.parse() {
				Ok(value) => visitor.$visit(value),
				Err(_) => Err(E),
			}
		}
	};
}

impl<'de> Deserializer<'de> for FromStr<'de> {
	type Error = E;

	fn deserialize_any<V>(self, visitor: V) -> Result<V::Value, Self::Error>
	where
		V: Visitor<'de>,
	{
		println!("from str: deserialize_any");
		Err(E)
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

	fn deserialize_char<V>(self, visitor: V) -> Result<V::Value, Self::Error>
	where
		V: Visitor<'de>,
	{
		println!("from str: deserialize_char");
		let value = self.0.ok_or(E)?;
		let mut chars = value.chars();
		let value = chars.next().ok_or(E)?;

		if chars.any(|remaining| true) {
			return Err(E);
		}

		visitor.visit_char(value)
	}

	fn deserialize_str<V>(self, visitor: V) -> Result<V::Value, Self::Error>
	where
		V: Visitor<'de>,
	{
		println!("from str: deserialize_str");
		let value = self.0.ok_or(E)?;

		visitor.visit_borrowed_str(value)
	}

	fn deserialize_string<V>(self, visitor: V) -> Result<V::Value, Self::Error>
	where
		V: Visitor<'de>,
	{
		println!("from str: deserialize_string");
		let value = self.0.ok_or(E)?;

		visitor.visit_string(value.to_owned())
	}

	fn deserialize_bytes<V>(self, visitor: V) -> Result<V::Value, Self::Error>
	where
		V: Visitor<'de>,
	{
		println!("from str: deserialize_bytes");
		let value = self.0.ok_or(E)?;

		visitor.visit_borrowed_bytes(value.as_bytes())
	}

	fn deserialize_byte_buf<V>(self, visitor: V) -> Result<V::Value, Self::Error>
	where
		V: Visitor<'de>,
	{
		println!("from str: deserialize_byte_buf");
		let value = self.0.ok_or(E)?;

		visitor.visit_byte_buf(value.as_bytes().to_owned())
	}

	fn deserialize_option<V>(self, visitor: V) -> Result<V::Value, Self::Error>
	where
		V: Visitor<'de>,
	{
		println!("from str: deserialize_option");
		if self.0.is_some() {
			visitor.visit_some(self)
		} else {
			visitor.visit_none()
		}
	}

	fn deserialize_unit<V>(self, visitor: V) -> Result<V::Value, Self::Error>
	where
		V: Visitor<'de>,
	{
		println!("from str: deserialize_unit");
		visitor.visit_unit()
	}

	fn deserialize_unit_struct<V>(
		self,
		_name: &'static str,
		visitor: V,
	) -> Result<V::Value, Self::Error>
	where
		V: Visitor<'de>,
	{
		println!("from str: deserialize_unit_struct");
		visitor.visit_unit()
	}

	fn deserialize_newtype_struct<V>(
		self,
		_name: &'static str,
		visitor: V,
	) -> Result<V::Value, Self::Error>
	where
		V: Visitor<'de>,
	{
		println!("from str: deserialize_newtype_struct");
		visitor.visit_newtype_struct(self)
	}

	fn deserialize_enum<V>(
		self,
		_name: &'static str,
		_variants: &'static [&'static str],
		visitor: V,
	) -> Result<V::Value, Self::Error>
	where
		V: Visitor<'de>,
	{
		println!("from str: deserialize_enum");
		visitor.visit_enum(self)
	}

	fn deserialize_identifier<V>(self, visitor: V) -> Result<V::Value, Self::Error>
	where
		V: Visitor<'de>,
	{
		println!("from str: deserialize_identifier");
		self.deserialize_str(visitor)
	}

	fn deserialize_ignored_any<V>(self, visitor: V) -> Result<V::Value, Self::Error>
	where
		V: Visitor<'de>,
	{
		println!("from str: deserialize_ignored_any");
		visitor.visit_unit()
	}

	forward_to_deserialize_any! { seq tuple tuple_struct map struct }
}

impl<'de> EnumAccess<'de> for FromStr<'de> {
	type Error = E;
	type Variant = Self;

	fn variant_seed<V>(mut self, seed: V) -> Result<(V::Value, Self::Variant), Self::Error>
	where
		V: DeserializeSeed<'de>,
	{
		println!("from str: variant_seed");
		seed.deserialize(self.clone()).map(|value| (value, self))
	}
}

impl<'de> VariantAccess<'de> for FromStr<'de> {
	type Error = E;

	fn unit_variant(self) -> Result<(), Self::Error> {
		println!("from str: unit_variant");
		Ok(())
	}

	fn newtype_variant_seed<T>(self, seed: T) -> Result<T::Value, Self::Error>
	where
		T: DeserializeSeed<'de>,
	{
		println!("from str: newtype_variant_seed");
		seed.deserialize(self)
	}

	fn tuple_variant<V>(self, _len: usize, visitor: V) -> Result<V::Value, Self::Error>
	where
		V: Visitor<'de>,
	{
		println!("from str: tuple_variant");
		Err(E)
	}

	fn struct_variant<V>(
		self,
		_fields: &'static [&'static str],
		visitor: V,
	) -> Result<V::Value, Self::Error>
	where
		V: Visitor<'de>,
	{
		println!("from str: struct_variant");
		Err(E)
	}
}

// --------------------------------------------------
