use std::{
	iter::{Map, Peekable},
	slice::Iter,
};

use serde::{
	de::{DeserializeSeed, EnumAccess, MapAccess, SeqAccess, VariantAccess, Visitor},
	Deserializer,
};

use crate::pattern::Params;

use super::{from_params::FromParams, DataType, DeserializerError};

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

#[allow(clippy::type_complexity)]
pub(crate) struct FromParamsList<'de> {
	data_type: DataType,
	params_deserializers_itr: Peekable<Map<Iter<'de, Params>, fn(&'de Params) -> FromParams<'de>>>,
}

impl<'de> FromParamsList<'de> {
	#[inline]
	pub(crate) fn new(params_list: &'de [Params]) -> Self {
		let into_from_segment: fn(&'de Params) -> FromParams<'de> = FromParams::new;

		Self {
			data_type: DataType::Unknown,
			params_deserializers_itr: params_list.iter().map(into_from_segment).peekable(),
		}
	}

	fn current_valid_params_deserializer(&mut self) -> Option<&mut FromParams<'de>> {
		loop {
			let some_deserializer = self.params_deserializers_itr.peek_mut();

			if some_deserializer.is_none() {
				break;
			}

			if some_deserializer.is_some_and(|from_params| from_params.is_valid()) {
				return self.params_deserializers_itr.peek_mut();
			}

			self.params_deserializers_itr.next(); // Advancing the iterator.
		}

		None
	}
}

// --------------------------------------------------

macro_rules! declare_deserialize_for_simple_types {
	($($deserialize:ident)*) => {
		$(
			fn $deserialize<V>(self, visitor: V) -> Result<V::Value, Self::Error>
			where
				V: Visitor<'de>,
			{
				if let Some(from_params) = self.current_valid_params_deserializer() {
					return from_params.$deserialize(visitor)
				}

				Err(DeserializerError::NoDataIsAvailable)
			}
		)*
	};
}

impl<'a, 'de> Deserializer<'de> for &'a mut FromParamsList<'de> {
	type Error = DeserializerError;

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

	fn deserialize_seq<V>(self, visitor: V) -> Result<V::Value, Self::Error>
	where
		V: Visitor<'de>,
	{
		self.data_type = DataType::Sequence;
		visitor.visit_seq(FromParamsListSeqAccess::new(self))
	}

	fn deserialize_tuple<V>(self, _len: usize, visitor: V) -> Result<V::Value, Self::Error>
	where
		V: Visitor<'de>,
	{
		self.data_type = DataType::Tuple;
		visitor.visit_seq(FromParamsListSeqAccess::new(self))
	}

	fn deserialize_tuple_struct<V>(
		self,
		_name: &'static str,
		_len: usize,
		visitor: V,
	) -> Result<V::Value, Self::Error>
	where
		V: Visitor<'de>,
	{
		self.data_type = DataType::Tuple;
		visitor.visit_seq(FromParamsListSeqAccess::new(self))
	}

	fn deserialize_map<V>(self, visitor: V) -> Result<V::Value, Self::Error>
	where
		V: Visitor<'de>,
	{
		self.data_type = DataType::Map;
		visitor.visit_map(FromParamsListMapAccess::new(self))
	}

	fn deserialize_struct<V>(
		self,
		_name: &'static str,
		_fields: &'static [&'static str],
		visitor: V,
	) -> Result<V::Value, Self::Error>
	where
		V: Visitor<'de>,
	{
		self.data_type = DataType::Struct;
		visitor.visit_map(FromParamsListMapAccess::new(self))
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
		visitor.visit_enum(FromParamsListEnumAccess::new(self))
	}
}

// -------------------------

struct FromParamsListSeqAccess<'a, 'de>(&'a mut FromParamsList<'de>);

impl<'a, 'de> FromParamsListSeqAccess<'a, 'de> {
	#[inline]
	fn new(from_params_list: &'a mut FromParamsList<'de>) -> Self {
		Self(from_params_list)
	}
}

impl<'de> SeqAccess<'de> for FromParamsListSeqAccess<'_, 'de> {
	type Error = DeserializerError;

	fn next_element_seed<T>(&mut self, seed: T) -> Result<Option<T::Value>, Self::Error>
	where
		T: DeserializeSeed<'de>,
	{
		let data_type = self.0.data_type;

		if let Some(from_params) = self.0.current_valid_params_deserializer() {
			from_params.set_high_level_data_type(data_type);

			return seed.deserialize(from_params).map(Some);
		}

		Ok(None)
	}
}

// -------------------------

struct FromParamsListMapAccess<'a, 'de>(&'a mut FromParamsList<'de>);

impl<'a, 'de> FromParamsListMapAccess<'a, 'de> {
	#[inline]
	fn new(from_params_list: &'a mut FromParamsList<'de>) -> Self {
		Self(from_params_list)
	}
}

impl<'de> MapAccess<'de> for FromParamsListMapAccess<'_, 'de> {
	type Error = DeserializerError;

	fn next_key_seed<K>(&mut self, seed: K) -> Result<Option<K::Value>, Self::Error>
	where
		K: DeserializeSeed<'de>,
	{
		let data_type = self.0.data_type;

		if let Some(from_params) = self.0.current_valid_params_deserializer() {
			from_params.set_high_level_data_type(data_type);

			return from_params.deserialize_map_key(seed);
		}

		Ok(None)
	}

	fn next_value_seed<V>(&mut self, seed: V) -> Result<V::Value, Self::Error>
	where
		V: DeserializeSeed<'de>,
	{
		let data_type = self.0.data_type;

		if let Some(from_params) = self.0.current_valid_params_deserializer() {
			from_params.set_high_level_data_type(data_type);

			return from_params.deserialize_map_value(seed);
		}

		Err(DeserializerError::NoDataIsAvailable)
	}
}

// -------------------------

struct FromParamsListEnumAccess<'a, 'de>(&'a mut FromParamsList<'de>);

impl<'a, 'de> FromParamsListEnumAccess<'a, 'de> {
	#[inline]
	fn new(from_params_list: &'a mut FromParamsList<'de>) -> Self {
		Self(from_params_list)
	}
}

impl<'de> EnumAccess<'de> for FromParamsListEnumAccess<'_, 'de> {
	type Error = DeserializerError;
	type Variant = Self;

	fn variant_seed<V>(self, seed: V) -> Result<(V::Value, Self::Variant), Self::Error>
	where
		V: DeserializeSeed<'de>,
	{
		let value = seed.deserialize(&mut *self.0)?;

		Ok((value, self))
	}
}

impl<'de> VariantAccess<'de> for FromParamsListEnumAccess<'_, 'de> {
	type Error = DeserializerError;

	fn unit_variant(self) -> Result<(), Self::Error> {
		Ok(())
	}

	fn newtype_variant_seed<T>(self, seed: T) -> Result<T::Value, Self::Error>
	where
		T: DeserializeSeed<'de>,
	{
		seed.deserialize(self.0)
	}

	fn tuple_variant<V>(self, _len: usize, visitor: V) -> Result<V::Value, Self::Error>
	where
		V: Visitor<'de>,
	{
		self.0.deserialize_seq(visitor)
	}

	fn struct_variant<V>(
		self,
		_fields: &'static [&'static str],
		visitor: V,
	) -> Result<V::Value, Self::Error>
	where
		V: Visitor<'de>,
	{
		self.0.deserialize_map(visitor)
	}
}

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

#[cfg(all(test, feature = "full"))]
mod test {
	use std::collections::HashMap;

	use serde::Deserialize;

	use crate::{
		pattern::{DeserializerError, ParamsList, Pattern},
		request::routing::RouteSegments,
	};

	// --------------------------------------------------

	#[test]
	fn deserialize_path_params() {
		let path = "/{wl_0_0}/{wl_1_0}/static-{rx_2_0p1:p1}_{rx_2_0p2:42}/{rx_3_0:cp3|42}";
		let mut patterns = Vec::new();
		for (segment, _) in RouteSegments::new(path) {
			patterns.push(Pattern::parse(segment))
		}

		let match_path = "/cba0/cba1/static-p1_42/42";

		let mut params_list = ParamsList::new();
		let mut patterns_iter = patterns.iter();

		for (match_segment, _) in RouteSegments::new(match_path) {
			let pattern = patterns_iter.next().unwrap();
			match pattern {
				Pattern::Static(_) => assert!(pattern.is_static_match(match_segment).is_some_and(|r| r)),
				Pattern::Regex(_, _) => assert!(pattern
					.is_regex_match(match_segment, &mut params_list)
					.is_some_and(|r| r),),
				Pattern::Wildcard(_) => assert!(pattern
					.is_wildcard_match(match_segment.into(), &mut params_list)
					.is_some_and(|r| r),),
			}
		}

		// --------------------------------------------------

		let value = Option::<&str>::deserialize(&mut params_list.deserializer()).unwrap();
		assert_eq!(value, Some("cba0"));

		// ----------

		let mut params_list_with_empty_value = ParamsList::new();
		assert!(Pattern::parse("{wilcard}")
			.is_wildcard_match("".into(), &mut params_list_with_empty_value)
			.is_some_and(|r| r));

		let some_value =
			Option::<&str>::deserialize(&mut params_list_with_empty_value.deserializer()).unwrap();

		assert!(some_value.is_none());

		// ----------

		let empty_params_list = ParamsList::new();
		let error = Option::<&str>::deserialize(&mut empty_params_list.deserializer()).unwrap_err();
		let DeserializerError::NoDataIsAvailable = error else {
			panic!("unexpected error: {}", error);
		};

		// -------------------------

		let values =
			<(&str, String, &str, u8, i32)>::deserialize(&mut params_list.deserializer()).unwrap();

		assert_eq!(values, ("cba0", "cba1".to_owned(), "p1", 42_u8, 42_i32));

		// -------------------------

		let values = <Vec<(&str, &str)>>::deserialize(&mut params_list.deserializer()).unwrap();
		assert_eq!(
			values,
			vec![
				("wl_0_0", "cba0"),
				("wl_1_0", "cba1"),
				("rx_2_0p1", "p1"),
				("rx_2_0p2", "42"),
				("rx_3_0", "42")
			],
		);

		// -------------------------

		let values = <HashMap<String, String>>::deserialize(&mut params_list.deserializer()).unwrap();
		let expected_values = [
			("wl_0_0", "cba0"),
			("wl_1_0", "cba1"),
			("rx_2_0p1", "p1"),
			("rx_2_0p2", "42"),
			("rx_3_0", "42"),
		]
		.iter()
		.fold(HashMap::new(), |mut map, tuple| {
			map.insert(tuple.0.to_owned(), tuple.1.to_owned());
			map
		});
		assert_eq!(values, expected_values);

		// -------------------------

		#[derive(Deserialize, PartialEq, Debug)]
		struct NewTuple<'a>(String, &'a str, &'a str, i16, u8);

		let values = NewTuple::deserialize(&mut params_list.deserializer()).unwrap();
		assert_eq!(
			values,
			NewTuple("cba0".to_owned(), "cba1", "p1", 42_i16, 42_u8)
		);

		// -------------------------

		#[derive(Deserialize, PartialEq, Debug)]
		struct NewStruct<'a> {
			wl_0_0: String,
			wl_1_0: &'a str,
			rx_2_0p1: &'a str,
			rx_2_0p2: Option<u8>,
			rx_3_0: i16,
		}

		let values = NewStruct::deserialize(&mut params_list.deserializer()).unwrap();
		assert_eq!(
			values,
			NewStruct {
				wl_0_0: "cba0".to_owned(),
				wl_1_0: "cba1",
				rx_2_0p1: "p1",
				rx_2_0p2: Some(42_u8),
				rx_3_0: 42_i16
			},
		);
	}
}
