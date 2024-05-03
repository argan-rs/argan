use serde::{
	de::{DeserializeSeed, EnumAccess, MapAccess, SeqAccess, VariantAccess, Visitor},
	Deserializer,
};

use crate::pattern::Params;

use super::{DataType, DeserializerError, FromStr};

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

#[derive(Debug)]
pub(super) struct FromParams<'de> {
	params: &'de Params,
	current_params_index: usize,
	data_type: DataType,
	high_level_data_type: DataType,
	state: State,
}

#[repr(u8)]
#[derive(Debug, Default)]
enum State {
	#[default]
	NewParam,
	ParamNameTaken,
	Finished,
}

impl<'de> FromParams<'de> {
	pub(super) fn new(params: &'de Params) -> Self {
		Self {
			params,
			current_params_index: 0,
			data_type: DataType::Unknown,
			high_level_data_type: DataType::Unknown,
			state: State::NewParam,
		}
	}

	pub(super) fn is_valid(&self) -> bool {
		match self.state {
			State::NewParam | State::ParamNameTaken => true,
			State::Finished => false,
		}
	}

	pub(super) fn current_valid_param_name(&mut self) -> Option<&'de str> {
		if let State::ParamNameTaken | State::Finished = self.state {
			return None;
		};

		match self.params {
			#[cfg(feature = "regex")]
			Params::Regex(regex_names, _, _) => {
				if let Some((name, _)) = regex_names.get(self.current_params_index) {
					self.state = State::ParamNameTaken;

					return Some(name);
				}

				unreachable!("state must have been changed to 'Finished' when the last value was taken")
			}
			Params::Wildcard(name, _) => {
				self.state = State::ParamNameTaken;

				Some(name)
			}
		}
	}

	pub(super) fn current_valid_param_value(&mut self) -> Option<Option<&'de str>> {
		if let State::Finished = self.state {
			return None;
		};

		match self.params {
			#[cfg(feature = "regex")]
			Params::Regex(regex_names, captures_locations, text) => {
				if let Some((_, index)) = regex_names.get(self.current_params_index) {
					if let Some((start, end)) = captures_locations.get(index) {
						let value = &text[start..end];
						let some_value = if value.is_empty() { None } else { Some(value) };

						self.current_params_index += 1;
						if self.current_params_index < regex_names.len() {
							self.state = State::NewParam;
						} else {
							self.state = State::Finished;
						}

						return Some(some_value);
					}
				}

				unreachable!("state must have been changed to 'Finished' when the last value was taken")
			}
			Params::Wildcard(_, text) => {
				self.state = State::Finished;

				Some(Some(text))
			}
		}
	}

	pub(super) fn set_data_type(&mut self, data_type: DataType) {
		self.data_type = data_type
	}

	pub(super) fn set_high_level_data_type(&mut self, high_level_data_type: DataType) {
		self.high_level_data_type = high_level_data_type
	}

	pub(super) fn deserialize_next_element_seed<S>(
		&mut self,
		seed: S,
	) -> Result<Option<S::Value>, DeserializerError>
	where
		S: DeserializeSeed<'de>,
	{
		let mut deserializer = FromParamsSeqAccess::new(self);

		deserializer.next_element_seed(seed)
	}

	pub(super) fn deserialize_map_key<S>(
		&mut self,
		seed: S,
	) -> Result<Option<S::Value>, DeserializerError>
	where
		S: DeserializeSeed<'de>,
	{
		let mut deserializer = FromParamsMapAccess::new(self);

		deserializer.next_key_seed(seed)
	}

	pub(super) fn deserialize_map_value<S>(&mut self, seed: S) -> Result<S::Value, DeserializerError>
	where
		S: DeserializeSeed<'de>,
	{
		let mut deserializer = FromParamsMapAccess::new(self);

		deserializer.next_value_seed(seed)
	}
}

// --------------------------------------------------

macro_rules! declare_deserialize_for_simple_types {
	($($deserialize:ident)*) => {
		$(
			#[inline]
			fn $deserialize<V>(self, visitor: V) -> Result<V::Value, Self::Error>
			where
				V: Visitor<'de>,
			{
				println!("[{}] from params: {}", line!(), stringify!($deserialize));
				self.current_valid_param_value().ok_or(DeserializerError::NoDataIsAvailable).and_then(
					|some_value| FromStr::new(some_value).$deserialize(visitor),
				)
			}
		)*
	};
}

impl<'de, 'a> Deserializer<'de> for &'a mut FromParams<'de> {
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
		println!("[{}] from params: deserialize_unit_struct", line!());
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
		println!("[{}] from params: deserialize_newtype_struct", line!());
		visitor.visit_newtype_struct(self)
	}

	fn deserialize_seq<V>(self, visitor: V) -> Result<V::Value, Self::Error>
	where
		V: Visitor<'de>,
	{
		println!("[{}] from params: deserialize_seq", line!());
		self.data_type = DataType::Sequence;
		visitor.visit_seq(FromParamsSeqAccess::new(self))
	}

	fn deserialize_tuple<V>(self, _len: usize, visitor: V) -> Result<V::Value, Self::Error>
	where
		V: Visitor<'de>,
	{
		println!("[{}] from params: deserialize_tuple", line!());
		self.data_type = DataType::Tuple;
		visitor.visit_seq(FromParamsSeqAccess::new(self))
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
		println!("[{}] from params: deserialize_tuple_struct", line!());
		self.data_type = DataType::Tuple;
		visitor.visit_seq(FromParamsSeqAccess::new(self))
	}

	fn deserialize_map<V>(self, visitor: V) -> Result<V::Value, Self::Error>
	where
		V: Visitor<'de>,
	{
		println!("[{}] from params: deserialize_map", line!());
		self.data_type = DataType::Map;
		visitor.visit_map(FromParamsMapAccess::new(self))
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
		println!("[{}] from params: deserialize_struct", line!());
		self.data_type = DataType::Struct;
		visitor.visit_map(FromParamsMapAccess::new(self))
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
		println!("[{}] from params: deserialize_enum", line!());
		visitor.visit_enum(FromParamsEnumAccess::new(self))
	}
}

// -------------------------

struct FromParamsSeqAccess<'a, 'de>(&'a mut FromParams<'de>);

impl<'a, 'de> FromParamsSeqAccess<'a, 'de> {
	#[inline]
	fn new(from_segment_params: &'a mut FromParams<'de>) -> Self {
		Self(from_segment_params)
	}
}

impl<'de, 'a> SeqAccess<'de> for FromParamsSeqAccess<'a, 'de> {
	type Error = DeserializerError;

	fn next_element_seed<T>(&mut self, seed: T) -> Result<Option<T::Value>, Self::Error>
	where
		T: DeserializeSeed<'de>,
	{
		println!("[{}] from params: next_element_seed", line!());
		if self.0.high_level_data_type == DataType::Sequence && self.0.data_type == DataType::Tuple {
			let some_name = self.0.current_valid_param_name();
			if some_name.is_some() {
				println!("[{}] name: {:?}", line!(), some_name);
				return seed.deserialize(FromStr::new(some_name)).map(Some);
			}
		}

		if let Some(some_value) = self.0.current_valid_param_value() {
			println!("[{}] value: {:?}", line!(), some_value);
			return seed.deserialize(FromStr::new(some_value)).map(Some);
		}

		Ok(None)
	}
}

// -------------------------

struct FromParamsMapAccess<'a, 'de>(&'a mut FromParams<'de>);

impl<'a, 'de> FromParamsMapAccess<'a, 'de> {
	#[inline]
	fn new(from_segment_params: &'a mut FromParams<'de>) -> Self {
		Self(from_segment_params)
	}
}

impl<'de, 'a> MapAccess<'de> for FromParamsMapAccess<'a, 'de> {
	type Error = DeserializerError;

	fn next_key_seed<K>(&mut self, seed: K) -> Result<Option<K::Value>, Self::Error>
	where
		K: DeserializeSeed<'de>,
	{
		println!("[{}] from params: next_key_seed", line!());
		let some_name = self.0.current_valid_param_name();
		if some_name.is_some() {
			println!("[{}] name: {:?}", line!(), some_name);

			return seed.deserialize(FromStr::new(some_name)).map(Some);
		}

		Ok(None)
	}

	fn next_value_seed<V>(&mut self, seed: V) -> Result<V::Value, Self::Error>
	where
		V: DeserializeSeed<'de>,
	{
		println!("[{}] from params: next_value_seed", line!());
		if let Some(some_value) = self.0.current_valid_param_value() {
			println!("[{}] value: {:?}", line!(), some_value);

			return seed.deserialize(FromStr::new(some_value));
		}

		Err(DeserializerError::NoDataIsAvailable)
	}
}

// -------------------------

struct FromParamsEnumAccess<'a, 'de>(&'a mut FromParams<'de>);

impl<'a, 'de> FromParamsEnumAccess<'a, 'de> {
	#[inline]
	fn new(params: &'a mut FromParams<'de>) -> Self {
		Self(params)
	}
}

impl<'de, 'a> EnumAccess<'de> for FromParamsEnumAccess<'a, 'de> {
	type Error = DeserializerError;
	type Variant = Self;

	fn variant_seed<V>(self, seed: V) -> Result<(V::Value, Self::Variant), Self::Error>
	where
		V: DeserializeSeed<'de>,
	{
		println!("[{}] from params: variant_seed", line!());
		let value = seed.deserialize(&mut *self.0)?;

		Ok((value, self))
	}
}

impl<'de, 'a> VariantAccess<'de> for FromParamsEnumAccess<'a, 'de> {
	type Error = DeserializerError;

	fn unit_variant(self) -> Result<(), Self::Error> {
		println!("[{}] from params: unit_variant", line!());
		Ok(())
	}

	fn newtype_variant_seed<T>(self, seed: T) -> Result<T::Value, Self::Error>
	where
		T: DeserializeSeed<'de>,
	{
		println!("[{}] from params: newtype_variant_seed", line!());
		seed.deserialize(self.0)
	}

	fn tuple_variant<V>(self, _len: usize, visitor: V) -> Result<V::Value, Self::Error>
	where
		V: Visitor<'de>,
	{
		println!("[{}] from params: tuple_variant", line!());
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
		println!("[{}] from params: struct_variant", line!());
		self.0.deserialize_map(visitor)
	}
}

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------
