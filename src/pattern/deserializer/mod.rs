use std::fmt::Display;

use argan_core::BoxedError;
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

// --------------------------------------------------
// FromStr

#[derive(Clone)]
struct FromStr<'de>(Option<&'de str>);

impl<'de> FromStr<'de> {
	#[inline]
	pub(super) fn new(some_str: Option<&'de str>) -> Self {
		Self(some_str)
	}
}

macro_rules! declare_deserialize_for_parsable {
	($deserialize:ident, $visit:ident, $type:ty) => {
		fn $deserialize<V>(self, visitor: V) -> Result<V::Value, Self::Error>
		where
			V: Visitor<'de>,
		{
			let value = self.0.ok_or(DeserializerError::NoDataIsAvailable)?;

			match value.parse() {
				Ok(value) => visitor.$visit(value),
				Err(error) => Err(DeserializerError::ParsingFailue(error.into())),
			}
		}
	};
}

impl<'de> Deserializer<'de> for FromStr<'de> {
	type Error = DeserializerError;

	fn deserialize_any<V>(self, _visitor: V) -> Result<V::Value, Self::Error>
	where
		V: Visitor<'de>,
	{
		Err(DeserializerError::UnsupportedType)
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
		let value = self.0.ok_or(DeserializerError::NoDataIsAvailable)?;
		let mut chars = value.chars();
		let value = chars.next().ok_or(DeserializerError::NoDataIsAvailable)?;

		if chars.any(|_remaining| true) {
			return Err(DeserializerError::NoDataIsAvailable);
		}

		visitor.visit_char(value)
	}

	fn deserialize_str<V>(self, visitor: V) -> Result<V::Value, Self::Error>
	where
		V: Visitor<'de>,
	{
		let value = self.0.ok_or(DeserializerError::NoDataIsAvailable)?;

		visitor.visit_borrowed_str(value)
	}

	fn deserialize_string<V>(self, visitor: V) -> Result<V::Value, Self::Error>
	where
		V: Visitor<'de>,
	{
		let value = self.0.ok_or(DeserializerError::NoDataIsAvailable)?;

		visitor.visit_string(value.to_owned())
	}

	fn deserialize_bytes<V>(self, visitor: V) -> Result<V::Value, Self::Error>
	where
		V: Visitor<'de>,
	{
		let value = self.0.ok_or(DeserializerError::NoDataIsAvailable)?;

		visitor.visit_borrowed_bytes(value.as_bytes())
	}

	fn deserialize_byte_buf<V>(self, visitor: V) -> Result<V::Value, Self::Error>
	where
		V: Visitor<'de>,
	{
		let value = self.0.ok_or(DeserializerError::NoDataIsAvailable)?;

		visitor.visit_byte_buf(value.as_bytes().to_owned())
	}

	fn deserialize_option<V>(self, visitor: V) -> Result<V::Value, Self::Error>
	where
		V: Visitor<'de>,
	{
		if self.0.is_none() {
			return Err(DeserializerError::NoDataIsAvailable);
		}

		if self.0.is_some_and(|value| !value.is_empty()) {
			visitor.visit_some(self)
		} else {
			visitor.visit_none()
		}
	}

	fn deserialize_unit<V>(self, visitor: V) -> Result<V::Value, Self::Error>
	where
		V: Visitor<'de>,
	{
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
		visitor.visit_enum(self)
	}

	fn deserialize_identifier<V>(self, visitor: V) -> Result<V::Value, Self::Error>
	where
		V: Visitor<'de>,
	{
		self.deserialize_str(visitor)
	}

	fn deserialize_ignored_any<V>(self, visitor: V) -> Result<V::Value, Self::Error>
	where
		V: Visitor<'de>,
	{
		visitor.visit_unit()
	}

	forward_to_deserialize_any! { seq tuple tuple_struct map struct }
}

impl<'de> EnumAccess<'de> for FromStr<'de> {
	type Error = DeserializerError;
	type Variant = Self;

	fn variant_seed<V>(self, seed: V) -> Result<(V::Value, Self::Variant), Self::Error>
	where
		V: DeserializeSeed<'de>,
	{
		seed.deserialize(self.clone()).map(|value| (value, self))
	}
}

impl<'de> VariantAccess<'de> for FromStr<'de> {
	type Error = DeserializerError;

	fn unit_variant(self) -> Result<(), Self::Error> {
		Ok(())
	}

	fn newtype_variant_seed<T>(self, seed: T) -> Result<T::Value, Self::Error>
	where
		T: DeserializeSeed<'de>,
	{
		seed.deserialize(self)
	}

	fn tuple_variant<V>(self, _len: usize, _visitor: V) -> Result<V::Value, Self::Error>
	where
		V: Visitor<'de>,
	{
		Err(DeserializerError::UnsupportedType)
	}

	fn struct_variant<V>(
		self,
		_fields: &'static [&'static str],
		_visitor: V,
	) -> Result<V::Value, Self::Error>
	where
		V: Visitor<'de>,
	{
		Err(DeserializerError::UnsupportedType)
	}
}

// --------------------------------------------------
// DataType

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
// DeserializerError

#[non_exhaustive]
#[derive(Debug, crate::ImplError)]
pub enum DeserializerError {
	#[error("{0}")]
	Message(String),
	#[error(transparent)]
	ParsingFailue(BoxedError),
	#[error("no data is available")]
	NoDataIsAvailable,
	#[error("unsupported type")]
	UnsupportedType,
}

impl serde::de::Error for DeserializerError {
	fn custom<T: Display>(message: T) -> Self {
		Self::Message(message.to_string())
	}
}

// --------------------------------------------------------------------------------
