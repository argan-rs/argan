use std::{
	iter::{Map, Peekable},
	slice::IterMut,
};

use serde::{
	de::{DeserializeSeed, EnumAccess, MapAccess, SeqAccess, VariantAccess, Visitor},
	Deserializer,
};

use crate::pattern::Params;

use super::{from_segment_params::FromSegmentParams, FromStr, Kind, E};

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

#[allow(clippy::type_complexity)]
pub(super) struct FromPathParams<'p, 'de> {
	kind: Kind,
	segment_params_list:
		Peekable<Map<IterMut<'p, Params<'de>>, fn(&'p mut Params<'de>) -> FromSegmentParams<'p, 'de>>>,
}

impl<'p, 'de> FromPathParams<'p, 'de> {
	#[inline]
	pub(super) fn new(segment_params_list: &'p mut [Params<'de>]) -> Self {
		let into_from_segment_params: fn(&'p mut Params<'de>) -> FromSegmentParams<'p, 'de> =
			FromSegmentParams::new;

		Self {
			kind: Kind::default(),
			segment_params_list: segment_params_list
				.iter_mut()
				.map(into_from_segment_params)
				.peekable(),
		}
	}

	pub(super) fn current_valid(&mut self) -> Option<&mut FromSegmentParams<'p, 'de>> {
		println!("from path params: current_valid");
		loop {
			let some_deserializer = self.segment_params_list.peek_mut();

			if some_deserializer.is_none() {
				break;
			}

			if some_deserializer
				.is_some_and(|from_segment_params| from_segment_params.current_valid().is_some())
			{
				return self.segment_params_list.peek_mut();
			}

			self.segment_params_list.next(); // Advancing the iterator.
		}

		None
	}
}

impl<'p, 'de> Iterator for FromPathParams<'p, 'de> {
	type Item = FromSegmentParams<'p, 'de>;

	fn next(&mut self) -> Option<Self::Item> {
		println!("from path params: next");
		self.segment_params_list.next()
	}
}

// --------------------------------------------------

macro_rules! declare_deserialize_for_simple_types {
	($($deserialize:ident)*) => {
		$(
			#[inline]
			fn $deserialize<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
				println!("\nfrom path params: {}", stringify!($deserialize));
				if let Some(mut from_segment_params) = self.current_valid() {
					return from_segment_params.$deserialize(visitor)
				}

				Err(E)
			}
		)*
	};
}

impl<'de> Deserializer<'de> for &mut FromPathParams<'_, 'de> {
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
		println!("\nfrom path params: deserialize_unit_struct");
		visitor.visit_unit()
	}

	fn deserialize_newtype_struct<V: Visitor<'de>>(
		self,
		_name: &'static str,
		visitor: V,
	) -> Result<V::Value, Self::Error> {
		println!("\nfrom path params: deserialize_newtype_struct");
		visitor.visit_newtype_struct(self)
	}

	fn deserialize_seq<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
		println!("\nfrom path params: deserialize_seq");
		self.kind = Kind::Sequence;
		visitor.visit_seq(FromPathParamsSeqAccess::new(self))
	}

	fn deserialize_tuple<V: Visitor<'de>>(
		self,
		_len: usize,
		visitor: V,
	) -> Result<V::Value, Self::Error> {
		println!("\nfrom path params: deserialize_tuple");
		self.kind = Kind::Tuple;
		visitor.visit_seq(FromPathParamsSeqAccess::new(self))
	}

	fn deserialize_tuple_struct<V: Visitor<'de>>(
		self,
		_name: &'static str,
		_len: usize,
		visitor: V,
	) -> Result<V::Value, Self::Error> {
		println!("\nfrom path params: deserialize_tuple_struct");
		self.kind = Kind::Tuple;
		visitor.visit_seq(FromPathParamsSeqAccess::new(self))
	}

	fn deserialize_map<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
		println!("\nfrom path params: deserialize_map");
		self.kind = Kind::Map;
		visitor.visit_map(FromPathParamsMapAccess::new(self))
	}

	fn deserialize_struct<V: Visitor<'de>>(
		self,
		_name: &'static str,
		_fields: &'static [&'static str],
		visitor: V,
	) -> Result<V::Value, Self::Error> {
		println!("\nfrom path params: deserialize_struct");
		self.kind = Kind::Struct;
		visitor.visit_map(FromPathParamsMapAccess::new(self))
	}

	fn deserialize_enum<V: Visitor<'de>>(
		self,
		_name: &'static str,
		_variants: &'static [&'static str],
		visitor: V,
	) -> Result<V::Value, Self::Error> {
		println!("\nfrom path params: deserialize_enum");
		visitor.visit_enum(FromPathParamsEnumAccess::new(self))
	}
}

// -------------------------

struct FromPathParamsSeqAccess<'a, 'p, 'de>(&'a mut FromPathParams<'p, 'de>);

impl<'a, 'p, 'de> FromPathParamsSeqAccess<'a, 'p, 'de> {
	#[inline]
	fn new(from_path_params: &'a mut FromPathParams<'p, 'de>) -> Self {
		Self(from_path_params)
	}
}

impl<'de> SeqAccess<'de> for FromPathParamsSeqAccess<'_, '_, 'de> {
	type Error = E;

	fn next_element_seed<T: DeserializeSeed<'de>>(
		&mut self,
		seed: T,
	) -> Result<Option<T::Value>, Self::Error> {
		println!("from path params: next_element_seed");
		let kind = self.0.kind;

		if let Some(mut from_segment_params) = self.0.current_valid() {
			from_segment_params.set_parent_kind(kind);

			return seed.deserialize(from_segment_params).map(Some);
		}

		Ok(None)
	}
}

// -------------------------

struct FromPathParamsMapAccess<'a, 'p, 'de>(&'a mut FromPathParams<'p, 'de>);

impl<'a, 'p, 'de> FromPathParamsMapAccess<'a, 'p, 'de> {
	#[inline]
	fn new(from_path_params: &'a mut FromPathParams<'p, 'de>) -> Self {
		Self(from_path_params)
	}
}

impl<'de> MapAccess<'de> for FromPathParamsMapAccess<'_, '_, 'de> {
	type Error = E;

	fn next_key_seed<K: DeserializeSeed<'de>>(
		&mut self,
		seed: K,
	) -> Result<Option<K::Value>, Self::Error> {
		println!("from path params: next_key_seed");
		let kind = self.0.kind;

		if let Some(mut from_segment_params) = self.0.current_valid() {
			from_segment_params.set_parent_kind(kind);
			println!("segment: {}", from_segment_params.segment_name());
			if kind == Kind::Struct {
				println!("\tstruct field key -> ");

				return seed
					.deserialize(FromStr::new(Some(from_segment_params.segment_name())))
					.map(Some);
			}

			if kind == Kind::Map {
				println!("\tmap key -> ");

				return from_segment_params.deserialize_map_keey(seed);
			}
		}

		Ok(None)
	}

	fn next_value_seed<V: DeserializeSeed<'de>>(&mut self, seed: V) -> Result<V::Value, Self::Error> {
		println!("from path params: next_value_seed");
		let kind = self.0.kind;

		if let Some(mut from_segment_params) = self.0.current_valid() {
			println!("segment: {}", from_segment_params.segment_name());
			if kind == Kind::Struct {
				println!("\tstruct value -> ");
				return seed.deserialize(from_segment_params);
			}

			if kind == Kind::Map {
				println!("\tmap value -> ");
				return from_segment_params.deserialize_map_value(seed);
			}
		}

		Err(E)
	}
}

// -------------------------

struct FromPathParamsEnumAccess<'a, 'p, 'de>(&'a mut FromPathParams<'p, 'de>);

impl<'a, 'p, 'de> FromPathParamsEnumAccess<'a, 'p, 'de> {
	#[inline]
	fn new(from_path_params: &'a mut FromPathParams<'p, 'de>) -> Self {
		Self(from_path_params)
	}
}

impl<'de> EnumAccess<'de> for FromPathParamsEnumAccess<'_, '_, 'de> {
	type Error = E;
	type Variant = Self;

	fn variant_seed<V: DeserializeSeed<'de>>(
		self,
		seed: V,
	) -> Result<(V::Value, Self::Variant), Self::Error> {
		println!("from path params: variant_seed");
		let value = seed.deserialize(self.0.by_ref())?;

		Ok((value, self))
	}
}

impl<'de> VariantAccess<'de> for FromPathParamsEnumAccess<'_, '_, 'de> {
	type Error = E;

	fn unit_variant(self) -> Result<(), Self::Error> {
		Ok(())
	}

	fn newtype_variant_seed<T: DeserializeSeed<'de>>(self, seed: T) -> Result<T::Value, Self::Error> {
		println!("from path params: newtype_variant_seed");
		seed.deserialize(self.0)
	}

	fn tuple_variant<V: Visitor<'de>>(
		self,
		_len: usize,
		visitor: V,
	) -> Result<V::Value, Self::Error> {
		println!("from path params: tuple_variant");
		self.0.deserialize_seq(visitor)
	}

	fn struct_variant<V: Visitor<'de>>(
		self,
		fields: &'static [&'static str],
		visitor: V,
	) -> Result<V::Value, Self::Error> {
		println!("from path params: struct_variant");
		self.0.deserialize_map(visitor)
	}
}

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

#[cfg(test)]
mod test {
	use std::collections::HashMap;

	use serde::Deserialize;

	use crate::{
		pattern::{MatchOutcome, Pattern},
		routing::RouteSegments,
	};

	use super::*;

	// --------------------------------------------------

	#[test]
	fn deserialize_path_params() {
		let path = "/*abc0/*abc1/$abc2:static-@cn1(cp1)_@cn2(42)/$abc3:@(cp3|42)";
		let mut patterns = Vec::new();
		for (segment, _) in RouteSegments::new(path) {
			patterns.push(Pattern::parse(segment))
		}

		let match_path = "/cba0/cba1/static-cp1_42/42";

		let mut get_path_params = || {
			let mut path_params = Vec::new();
			let mut patterns_iter = patterns.iter();

			for (match_segment, _) in RouteSegments::new(match_path) {
				let outcome = patterns_iter.next().unwrap().is_match(match_segment);
				let params = match outcome {
					MatchOutcome::Dynamic(params) => params,
					_ => panic!("non-dynamic match"),
				};

				path_params.push(params);
			}

			path_params
		};

		let mut path_params = get_path_params();
		let values =
			<(&str, String, &str, u8, i32)>::deserialize(&mut FromPathParams::new(&mut path_params))
				.unwrap();
		assert_eq!(values, ("cba0", "cba1".to_owned(), "cp1", 42_u8, 42_i32));

		let mut path_params = get_path_params();
		let values =
			<Vec<(&str, &str)>>::deserialize(&mut FromPathParams::new(&mut path_params)).unwrap();
		assert_eq!(
			values,
			vec![
				("abc0", "cba0"),
				("abc1", "cba1"),
				("cn1", "cp1"),
				("cn2", "42"),
				("abc3", "42")
			]
		);

		let mut path_params = get_path_params();
		let values =
			<HashMap<String, String>>::deserialize(&mut FromPathParams::new(&mut path_params)).unwrap();
		let expected_values = [
			("abc0", "cba0"),
			("abc1", "cba1"),
			("cn1", "cp1"),
			("cn2", "42"),
			("abc3", "42"),
		]
		.iter()
		.fold(HashMap::new(), |mut map, tuple| {
			map.insert(tuple.0.to_owned(), tuple.1.to_owned());
			map
		});
		assert_eq!(values, expected_values);

		#[derive(Deserialize, PartialEq, Debug)]
		struct NewTuple<'a>(String, &'a str, &'a str, i16, u8);
		let mut path_params = get_path_params();
		let values = NewTuple::deserialize(&mut FromPathParams::new(&mut path_params)).unwrap();
		assert_eq!(
			values,
			NewTuple("cba0".to_owned(), "cba1", "cp1", 42_i16, 42_u8)
		);

		#[derive(Deserialize, PartialEq, Debug)]
		struct NewStruct<'a> {
			abc0: String,
			abc1: &'a str,
			abc2: (&'a str, u8),
			abc3: i16,
		}
		let mut path_params = get_path_params();
		let values = NewStruct::deserialize(&mut FromPathParams::new(&mut path_params)).unwrap();
		assert_eq!(
			values,
			NewStruct {
				abc0: "cba0".to_owned(),
				abc1: "cba1",
				abc2: ("cp1", 42_u8),
				abc3: 42_i16
			}
		);
	}
}
