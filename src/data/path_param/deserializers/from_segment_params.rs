use std::iter::{Map, Peekable};

use serde::{
	de::{DeserializeSeed, EnumAccess, MapAccess, SeqAccess, VariantAccess, Visitor},
	Deserializer,
};

use crate::pattern::{Param, Params};

use super::{from_param::FromParam, Kind, E};

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

#[allow(clippy::type_complexity)]
pub(super) struct FromSegmentParams<'p, 'de> {
	segment_name: &'de str,
	params: Peekable<Map<&'p mut Params<'de>, fn(Param<'de>) -> FromParam<'de>>>,
	kind: Kind,
	parent_kind: Kind,
}

impl<'p, 'de> FromSegmentParams<'p, 'de> {
	pub(super) fn new(segment_params: &'p mut Params<'de>) -> Self {
		let into_from_param: fn(Param<'de>) -> FromParam<'de> = FromParam::new;

		Self {
			segment_name: segment_params.name(),
			params: segment_params.map(into_from_param).peekable(),
			kind: Kind::default(),
			parent_kind: Kind::default(),
		}
	}

	pub(super) fn set_parent_kind(&mut self, parent_kind: Kind) {
		self.parent_kind = parent_kind
	}

	pub(super) fn segment_name(&self) -> &'de str {
		self.segment_name
	}

	pub(super) fn current_valid(&mut self) -> Option<&mut FromParam<'de>> {
		println!("from segment params: current_valid");
		loop {
			let some_deserializer = self.params.peek_mut();

			if some_deserializer.is_none() {
				break;
			}

			if some_deserializer.is_some_and(|from_param| from_param.is_valid()) {
				return self.params.peek_mut();
			}

			self.params.next(); // Advancing the iterator.
		}

		None
	}

	pub(super) fn deserialize_map_keey<K: DeserializeSeed<'de>>(
		&mut self,
		seed: K,
	) -> Result<Option<K::Value>, E> {
		let mut deserializer = FromSegmentParamsMapAccess::new(self);

		deserializer.next_key_seed(seed)
	}

	pub(super) fn deserialize_map_value<K: DeserializeSeed<'de>>(
		&mut self,
		seed: K,
	) -> Result<K::Value, E> {
		let mut deserializer = FromSegmentParamsMapAccess::new(self);

		deserializer.next_value_seed(seed)
	}
}

impl<'p, 'de> Iterator for FromSegmentParams<'p, 'de> {
	type Item = FromParam<'de>;

	#[inline]
	fn next(&mut self) -> Option<Self::Item> {
		println!("from segment params: next");
		self.params.next()
	}
}

// --------------------------------------------------

macro_rules! declare_deserialize_for_simple_types {
	($($deserialize:ident)*) => {
		$(
			#[inline]
			fn $deserialize<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
				println!("from segment params: {}", stringify!($deserialize));
				self.current_valid().ok_or(E).and_then(|from_param| from_param.$deserialize(visitor))
			}
		)*
	};
}

impl<'p, 'de> Deserializer<'de> for &mut FromSegmentParams<'p, 'de> {
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
		println!("from segment params: deserialize_unit_struct");
		visitor.visit_unit()
	}

	fn deserialize_newtype_struct<V: Visitor<'de>>(
		self,
		_name: &'static str,
		visitor: V,
	) -> Result<V::Value, Self::Error> {
		println!("from segment params: deserialize_newtype_struct");
		visitor.visit_newtype_struct(self)
	}

	fn deserialize_seq<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
		println!("from segment params: deserialize_seq");
		self.kind = Kind::Sequence;
		visitor.visit_seq(FromSegmentParamsSeqAccess::new(self))
	}

	fn deserialize_tuple<V: Visitor<'de>>(
		self,
		_len: usize,
		visitor: V,
	) -> Result<V::Value, Self::Error> {
		println!("from segment params: deserialize_tuple");
		self.kind = Kind::Tuple;
		visitor.visit_seq(FromSegmentParamsSeqAccess::new(self))
	}

	fn deserialize_tuple_struct<V: Visitor<'de>>(
		self,
		_name: &'static str,
		_len: usize,
		visitor: V,
	) -> Result<V::Value, Self::Error> {
		println!("from segment params: deserialize_tuple_struct");
		self.kind = Kind::Tuple;
		visitor.visit_seq(FromSegmentParamsSeqAccess::new(self))
	}

	fn deserialize_map<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
		println!("from segment params: deserialize_map");
		self.kind = Kind::Map;
		visitor.visit_map(FromSegmentParamsMapAccess::new(self))
	}

	fn deserialize_struct<V: Visitor<'de>>(
		self,
		_name: &'static str,
		_fields: &'static [&'static str],
		visitor: V,
	) -> Result<V::Value, Self::Error> {
		println!("from segment params: deserialize_struct");
		self.kind = Kind::Struct;
		visitor.visit_map(FromSegmentParamsMapAccess::new(self))
	}

	fn deserialize_enum<V: Visitor<'de>>(
		self,
		_name: &'static str,
		_variants: &'static [&'static str],
		visitor: V,
	) -> Result<V::Value, Self::Error> {
		println!("from segment params: deserialize_enum");
		visitor.visit_enum(FromSegmentParamsEnumAccess::new(self))
	}
}

// -------------------------

struct FromSegmentParamsSeqAccess<'a, 'p, 'de>(&'a mut FromSegmentParams<'p, 'de>);

impl<'a, 'p, 'de> FromSegmentParamsSeqAccess<'a, 'p, 'de> {
	#[inline]
	fn new(from_segment_params: &'a mut FromSegmentParams<'p, 'de>) -> Self {
		Self(from_segment_params)
	}
}

impl<'de> SeqAccess<'de> for FromSegmentParamsSeqAccess<'_, '_, 'de> {
	type Error = E;

	fn next_element_seed<T: DeserializeSeed<'de>>(
		&mut self,
		seed: T,
	) -> Result<Option<T::Value>, Self::Error> {
		println!("from segment params: next_element_seed");
		if self.0.parent_kind == Kind::Sequence && self.0.kind == Kind::Tuple {
			if let Some(mut from_param) = self.0.current_valid() {
				println!("from segment params: param: {:?}", from_param);

				return from_param.next_element_seed(seed);
			}
		} else if let Some(from_param) = self.0.current_valid() {
			return seed.deserialize(from_param).map(Some);
		}

		Ok(None)
	}
}

// -------------------------

struct FromSegmentParamsMapAccess<'a, 'p, 'de>(&'a mut FromSegmentParams<'p, 'de>);

impl<'a, 'p, 'de> FromSegmentParamsMapAccess<'a, 'p, 'de> {
	#[inline]
	fn new(from_segment_params: &'a mut FromSegmentParams<'p, 'de>) -> Self {
		Self(from_segment_params)
	}
}

impl<'de> MapAccess<'de> for FromSegmentParamsMapAccess<'_, '_, 'de> {
	type Error = E;

	fn next_key_seed<K: DeserializeSeed<'de>>(
		&mut self,
		seed: K,
	) -> Result<Option<K::Value>, Self::Error> {
		println!("from segment params: next_key_seed");
		if let Some(mut from_param) = self.0.current_valid() {
			println!("key of: {:?}", from_param);

			return from_param.next_key_seed(seed);
		}

		Ok(None)
	}

	fn next_value_seed<V: DeserializeSeed<'de>>(&mut self, seed: V) -> Result<V::Value, Self::Error> {
		println!("from segment params: next_value_seed");
		if let Some(mut from_param) = self.0.current_valid() {
			println!("value: {:?}", from_param);

			return from_param.next_value_seed(seed);
		}

		Err(E)
	}
}

// -------------------------

struct FromSegmentParamsEnumAccess<'a, 'p, 'de>(&'a mut FromSegmentParams<'p, 'de>);

impl<'a, 'p, 'de> FromSegmentParamsEnumAccess<'a, 'p, 'de> {
	#[inline]
	fn new(from_segment_params: &'a mut FromSegmentParams<'p, 'de>) -> Self {
		Self(from_segment_params)
	}
}

impl<'de> EnumAccess<'de> for FromSegmentParamsEnumAccess<'_, '_, 'de> {
	type Error = E;
	type Variant = Self;

	fn variant_seed<V: DeserializeSeed<'de>>(
		self,
		seed: V,
	) -> Result<(V::Value, Self::Variant), Self::Error> {
		println!("from segment params: variant_seed");
		let value = seed.deserialize(self.0.by_ref())?;

		Ok((value, self))
	}
}

impl<'de> VariantAccess<'de> for FromSegmentParamsEnumAccess<'_, '_, 'de> {
	type Error = E;

	fn unit_variant(self) -> Result<(), Self::Error> {
		println!("from segment params: unit_variant");
		Ok(())
	}

	fn newtype_variant_seed<T: DeserializeSeed<'de>>(self, seed: T) -> Result<T::Value, Self::Error> {
		println!("from segment params: newtype_variant_seed");
		seed.deserialize(self.0)
	}

	fn tuple_variant<V: Visitor<'de>>(
		self,
		_len: usize,
		visitor: V,
	) -> Result<V::Value, Self::Error> {
		println!("from segment params: tuple_variant");
		self.0.deserialize_seq(visitor)
	}

	fn struct_variant<V: Visitor<'de>>(
		self,
		_fields: &'static [&'static str],
		visitor: V,
	) -> Result<V::Value, Self::Error> {
		println!("from segment params: struct_variant");
		self.0.deserialize_map(visitor)
	}
}

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------
