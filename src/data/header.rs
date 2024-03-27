use std::{convert::Infallible, io::BufRead, num::ParseFloatError};

use tokio::io::AsyncBufReadExt;

use crate::{
	common::{marker::Sealed, trim, SCOPE_VALIDITY},
	handler::Args,
	request::{FromRequest, FromRequestHead, Request, RequestHead},
	response::{BoxedErrorResponse, IntoResponse, IntoResponseHead, Response, ResponseHead},
	ImplError,
};

// ----------

pub use http::header::*;

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

// --------------------------------------------------------------------------------

pub(crate) fn content_type<B>(request: &Request<B>) -> Result<&str, ContentTypeError> {
	let content_type = request
		.headers()
		.get(CONTENT_TYPE)
		.ok_or(ContentTypeError::Missing)?;

	content_type.to_str().map_err(Into::into)
}

#[derive(Debug, ImplError)]
pub(crate) enum ContentTypeError {
	#[error("missing Content-Type")]
	Missing,
	#[error(transparent)]
	InvalidValue(#[from] ToStrError),
}

// --------------------------------------------------------------------------------

pub(crate) trait HeaderMapExt: Sealed {
	fn has_header_with_value<N, V>(&self, header_name: N, value: V) -> Option<bool>
	where
		N: AsHeaderName,
		V: AsRef<[u8]>;

	fn get_all_split<N: AsHeaderName>(&self, header_name: N) -> impl Iterator<Item = &[u8]>;
}

impl HeaderMapExt for HeaderMap {
	fn has_header_with_value<N, V>(&self, header_name: N, value: V) -> Option<bool>
	where
		N: AsHeaderName,
		V: AsRef<[u8]>,
	{
		let header_values = self.get_all(header_name);
		let value = value.as_ref();

		let mut found = None;

		for header_value in header_values {
			if header_value
				.as_bytes()
				.split(|ch| *ch == b',')
				.map(trim)
				.any(|header_value| header_value.eq_ignore_ascii_case(value))
			{
				return Some(true);
			}

			found = Some(false);
		}

		found
	}

	fn get_all_split<N: AsHeaderName>(&self, header_name: N) -> impl Iterator<Item = &[u8]> {
		self
			.get_all(header_name)
			.into_iter()
			.flat_map(|header_value| header_value.as_bytes().split(|ch| *ch == b',').map(trim))
	}
}

impl Sealed for HeaderMap {}

// --------------------------------------------------

#[inline]
pub(crate) fn header_value_has_value<V: AsRef<[u8]>>(header_value: &HeaderValue, value: V) -> bool {
	if header_value
		.as_bytes()
		.split(|ch| *ch == b',')
		.map(trim)
		.any(|header_value| header_value.eq_ignore_ascii_case(value.as_ref()))
	{
		return true;
	}

	false
}

// ----------

pub(crate) fn split_header_value(
	header_value: &HeaderValue,
) -> Result<impl Iterator<Item = &str>, ToStrError> {
	Ok(header_value.to_str()?.split(',').filter_map(|value| {
		if value.is_empty() {
			return None;
		}

		Some(value.trim())
	}))
}

// ----------

pub(crate) fn split_header_value_with_weights(
	header_value: &HeaderValue,
) -> Result<Vec<(&str, f32)>, SplitHeaderValueError> {
	header_value
		.to_str()?
		.split(',')
		.try_fold::<_, _, Result<_, SplitHeaderValueError>>(Vec::new(), |mut values, value| {
			let value = value.trim().split_once(';').map_or(
				Result::<_, SplitHeaderValueError>::Ok((value, 1f32)),
				|segments| {
					let value = segments.0.trim_end();
					let quality = segments
						.1
						.trim_start()
						.strip_prefix("q=")
						.ok_or(SplitHeaderValueError::InvalidQualitySpecifier)?;

					let quality = quality.parse::<f32>()?;

					Ok((value, quality))
				},
			)?;

			values.push(value);

			Ok(values)
		})
		.map(|mut values| {
			values.sort_by(|a, b| b.1.partial_cmp(&a.1).expect(SCOPE_VALIDITY));

			values
		})
}

#[derive(Debug, crate::ImplError)]
pub enum SplitHeaderValueError {
	#[error(transparent)]
	ToStrError(#[from] ToStrError),
	#[error("invalid quality specifier")]
	InvalidQualitySpecifier,
	#[error(transparent)]
	ParseFloatError(#[from] ParseFloatError),
}

// --------------------------------------------------------------------------------

#[cfg(test)]
mod tempt_test {
	use http::{header, HeaderMap, HeaderValue};

	use super::HeaderMapExt;

	#[test]
	fn header_map_ext() {
		let mut header_map = HeaderMap::new();

		header_map.insert(
			header::CONNECTION,
			HeaderValue::from_static("value-1, value-2"),
		);

		header_map.append(header::CONNECTION, HeaderValue::from_static("value-3"));

		let cases = [b"value-1", b"value-2", b"value-3"];

		for (i, value) in header_map.get_all_split(header::CONNECTION).enumerate() {
			assert_eq!(value, cases[i]);
		}
	}
}
