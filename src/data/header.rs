//! HTTP header types.

// ----------

use std::num::ParseFloatError;

use argan_core::request::RequestHeadParts;

use crate::{
	common::{trim, SCOPE_VALIDITY},
	ImplError,
};

// ----------

pub use http::header::*;

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

// --------------------------------------------------------------------------------

pub(crate) fn content_type(head_parts: &RequestHeadParts) -> Result<&str, ContentTypeError> {
	let content_type = head_parts
		.headers
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
			// Sort in descending order.
			values.sort_by(|a, b| b.1.partial_cmp(&a.1).expect(SCOPE_VALIDITY));

			values
		})
}

#[derive(Debug, crate::ImplError)]
pub(crate) enum SplitHeaderValueError {
	#[error(transparent)]
	ToStrError(#[from] ToStrError),
	#[error("invalid quality specifier")]
	InvalidQualitySpecifier,
	#[error(transparent)]
	ParseFloatError(#[from] ParseFloatError),
}

// --------------------------------------------------------------------------------
