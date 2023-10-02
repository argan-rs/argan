use serde::{
	de::{DeserializeSeed, EnumAccess, MapAccess, SeqAccess, Visitor},
	forward_to_deserialize_any, Deserializer,
};

use crate::pattern::Param;

use super::{FromStr, E};

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

#[derive(Debug)]
pub(crate) struct FromParam<'de> {
	some_name: Option<&'de str>,
	some_option_value: Option<Option<&'de str>>, // To support optional values we need double Option.
}

impl<'de> FromParam<'de> {
	#[inline]
	pub(super) fn new(param: Param<'de>) -> Self {
		Self {
			some_name: Some(param.name()),
			some_option_value: Some(param.value()),
		}
	}

	pub(super) fn is_valid(&self) -> bool {
		self.some_option_value.is_some()
	}
}

// --------------------------------------------------

macro_rules! declare_deserialize_for_simple_types {
	($($deserialize:ident)*) => {
		$(
			fn $deserialize<V>(mut self, visitor: V) -> Result<V::Value, Self::Error>
			where
				V: Visitor<'de>,
			{
				println!("from param: {}", stringify!($deserialize));
				let some_value = self.some_option_value.take().ok_or(E)?;

				FromStr(some_value).$deserialize(visitor)
			}
		)*
	};
}

impl<'de> Deserializer<'de> for &mut FromParam<'de> {
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

	fn deserialize_unit_struct<V>(
		self,
		_name: &'static str,
		visitor: V,
	) -> Result<V::Value, Self::Error>
	where
		V: Visitor<'de>,
	{
		println!("from param: deserialize_unit_struct");
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
		println!("from param: deserialize_newtype_struct");
		visitor.visit_newtype_struct(self)
	}

	fn deserialize_tuple<V>(self, len: usize, visitor: V) -> Result<V::Value, Self::Error>
	where
		V: Visitor<'de>,
	{
		println!("from param: deserialize_tuple");
		if len < 3 {
			return visitor.visit_seq(self);
		}

		Err(E)
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
		println!("from param: deserialize_enum");
		visitor.visit_enum(self)
	}

	forward_to_deserialize_any! { seq tuple_struct map struct }
}

impl<'de> SeqAccess<'de> for FromParam<'de> {
	type Error = E;

	fn next_element_seed<T>(&mut self, seed: T) -> Result<Option<T::Value>, Self::Error>
	where
		T: DeserializeSeed<'de>,
	{
		println!("from param: next_element_seed");
		if self.some_name.is_some() {
			return seed.deserialize(FromStr(self.some_name.take())).map(Some);
		}

		if let Some(some_value) = self.some_option_value.take() {
			return seed.deserialize(FromStr(some_value)).map(Some);
		}

		Ok(None)
	}
}

impl<'de> MapAccess<'de> for FromParam<'de> {
	type Error = E;

	fn next_key_seed<K>(&mut self, seed: K) -> Result<Option<K::Value>, Self::Error>
	where
		K: DeserializeSeed<'de>,
	{
		println!("from param: next_key_seed");
		if self.some_name.is_some() {
			return seed.deserialize(FromStr(self.some_name.take())).map(Some);
		}

		Ok(None)
	}

	fn next_value_seed<V>(&mut self, seed: V) -> Result<V::Value, Self::Error>
	where
		V: DeserializeSeed<'de>,
	{
		println!("from param: next_value_seed");
		if let Some(some_value) = self.some_option_value.take() {
			return seed.deserialize(FromStr(some_value));
		}

		Err(E)
	}
}

impl<'de> EnumAccess<'de> for &mut FromParam<'de> {
	type Error = E;
	type Variant = FromStr<'de>;

	fn variant_seed<V>(self, seed: V) -> Result<(V::Value, Self::Variant), Self::Error>
	where
		V: DeserializeSeed<'de>,
	{
		let some_value = self.some_option_value.ok_or(E)?;
		let mut deserializer = FromStr(some_value);

		seed
			.deserialize(deserializer.clone())
			.map(|value| (value, deserializer))
	}
}

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

// #[cfg(test)]
// mod test {
// 	use std::ffi::CString;
//
// 	use serde::Deserialize;
//
// 	use super::*;
//
// 	// --------------------------------------------------
//
// 	#[test]
// 	fn deserialize_param() {
// 		let mut param = Param::new("abc", Some("5"));
// 		let value = i8::deserialize(FromParam::new(param)).unwrap();
// 		assert_eq!(value, 5_i8);
//
// 		param = Param::new("abc", Some("255"));
// 		let value = u8::deserialize(FromParam::new(param)).unwrap();
// 		assert_eq!(value, 255_u8);
//
// 		let result = i8::deserialize(FromParam::new(param));
// 		assert!(result.is_err());
//
// 		param = Param::new("abc", Some("-1000000000"));
// 		let value = i32::deserialize(FromParam::new(param)).unwrap();
// 		assert_eq!(value, -1_000_000_000_i32);
//
// 		let result = u32::deserialize(FromParam::new(param));
// 		assert!(result.is_err());
//
// 		param = Param::new("abc", Some("0.42"));
// 		let value = f64::deserialize(FromParam::new(param)).unwrap();
// 		assert_eq!(value, 0.42_f64);
//
// 		param = Param::new("abc", Some("x"));
// 		let value = char::deserialize(FromParam::new(param)).unwrap();
// 		assert_eq!(value, 'x');
//
// 		param = Param::new("abc", Some("xyz"));
// 		let result = char::deserialize(FromParam::new(param));
// 		assert!(result.is_err());
//
// 		let value = String::deserialize(FromParam::new(param)).unwrap();
// 		assert_eq!(value, "xyz");
//
// 		let value = <&[u8]>::deserialize(FromParam::new(param)).unwrap();
// 		assert_eq!(value, b"xyz");
//
// 		let value = <CString>::deserialize(FromParam::new(param)).unwrap();
// 		assert_eq!(value.as_bytes(), b"xyz");
//
// 		param = Param::new("abc", None);
// 		let value = <Option<bool>>::deserialize(FromParam::new(param)).unwrap();
// 		assert!(value.is_none());
//
// 		param = Param::new("abc", Some("42"));
// 		let value = <Option<i64>>::deserialize(FromParam::new(param)).unwrap();
// 		assert_eq!(value, Some(42_i64));
//
// 		#[derive(Deserialize)]
// 		struct Int(usize);
// 		let value = Int::deserialize(FromParam::new(param)).unwrap();
// 		assert_eq!(value.0, 42_usize);
//
// 		let value = <(&str, u16)>::deserialize(FromParam::new(param)).unwrap();
// 		assert_eq!(value, ("abc", 42_u16));
// 	}
// }
