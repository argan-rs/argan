use serde::{
	de::{DeserializeSeed, EnumAccess, MapAccess, SeqAccess, Visitor},
	forward_to_deserialize_any, Deserializer,
};

use super::{FromStr, E};

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

#[derive(Debug)]
pub(super) struct FromParam<'de> {
	some_name: Option<&'de str>,
	some_option_value: Option<Option<&'de str>>,
}

impl<'r> FromParam<'r> {
	#[inline]
	pub(super) fn new(some_name: Option<&'r str>, some_value: Option<&'r str>) -> Self {
		Self {
			some_name,
			some_option_value: Some(some_value),
		}
	}

	#[inline]
	pub(super) fn some_name(&mut self) -> Option<&'r str> {
		self.some_name.take()
	}

	#[inline]
	pub(super) fn some_option_value(&mut self) -> Option<Option<&'r str>> {
		self.some_option_value.take()
	}

	#[inline]
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
				let some_value = self.some_option_value().ok_or(E)?;

				FromStr::new(some_value).$deserialize(visitor)
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
		if let Some(name) = self.some_name() {
			return seed.deserialize(FromStr::new(Some(name))).map(Some);
		}

		if let Some(some_value) = self.some_option_value() {
			return seed.deserialize(FromStr::new(some_value)).map(Some);
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
		if let Some(name) = self.some_name() {
			return seed.deserialize(FromStr::new(Some(name))).map(Some);
		}

		Ok(None)
	}

	fn next_value_seed<V>(&mut self, seed: V) -> Result<V::Value, Self::Error>
	where
		V: DeserializeSeed<'de>,
	{
		println!("from param: next_value_seed");
		if let Some(some_value) = self.some_option_value() {
			return seed.deserialize(FromStr::new(some_value));
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
		let some_value = self.some_option_value().ok_or(E)?;
		let mut deserializer = FromStr::new(some_value);

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
// 		let value = i8::deserialize(&mut FromParam::new(param)).unwrap();
// 		assert_eq!(value, 5_i8);
//
// 		param = Param::new("abc", Some("255"));
// 		let value = u8::deserialize(&mut FromParam::new(param)).unwrap();
// 		assert_eq!(value, 255_u8);
//
// 		let result = i8::deserialize(&mut FromParam::new(param));
// 		assert!(result.is_err());
//
// 		param = Param::new("abc", Some("-1000000000"));
// 		let value = i32::deserialize(&mut FromParam::new(param)).unwrap();
// 		assert_eq!(value, -1_000_000_000_i32);
//
// 		let result = u32::deserialize(&mut FromParam::new(param));
// 		assert!(result.is_err());
//
// 		param = Param::new("abc", Some("0.42"));
// 		let value = f64::deserialize(&mut FromParam::new(param)).unwrap();
// 		assert_eq!(value, 0.42_f64);
//
// 		param = Param::new("abc", Some("x"));
// 		let value = char::deserialize(&mut FromParam::new(param)).unwrap();
// 		assert_eq!(value, 'x');
//
// 		param = Param::new("abc", Some("xyz"));
// 		let result = char::deserialize(&mut FromParam::new(param));
// 		assert!(result.is_err());
//
// 		let value = String::deserialize(&mut FromParam::new(param)).unwrap();
// 		assert_eq!(value, "xyz");
//
// 		let value = <&[u8]>::deserialize(&mut FromParam::new(param)).unwrap();
// 		assert_eq!(value, b"xyz");
//
// 		let value = <CString>::deserialize(&mut FromParam::new(param)).unwrap();
// 		assert_eq!(value.as_bytes(), b"xyz");
//
// 		param = Param::new("abc", None);
// 		let value = <Option<bool>>::deserialize(&mut FromParam::new(param)).unwrap();
// 		assert!(value.is_none());
//
// 		param = Param::new("abc", Some("42"));
// 		let value = <Option<i64>>::deserialize(&mut FromParam::new(param)).unwrap();
// 		assert_eq!(value, Some(42_i64));
//
// 		#[derive(Deserialize)]
// 		struct Int(usize);
// 		let value = Int::deserialize(&mut FromParam::new(param)).unwrap();
// 		assert_eq!(value.0, 42_usize);
//
// 		let value = <(&str, u16)>::deserialize(&mut FromParam::new(param)).unwrap();
// 		assert_eq!(value, ("abc", 42_u16));
// 	}
// }
