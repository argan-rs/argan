use std::{convert::Infallible, num::ParseFloatError};

use crate::{
	common::SCOPE_VALIDITY,
	handler::Args,
	request::{FromRequest, FromRequestHead, Request, RequestHead},
	response::{IntoResponse, IntoResponseHead, Response, ResponseHead},
	ImplError,
};

// ----------

pub use http::header::*;

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

impl<E: Sync> FromRequestHead<E> for HeaderMap {
	type Error = Infallible;

	async fn from_request_head(
		head: &mut RequestHead,
		_args: &mut Args<'_, E>,
	) -> Result<Self, Self::Error> {
		Ok(head.headers.clone())
	}
}

impl<B, E> FromRequest<B, E> for HeaderMap
where
	B: Send,
	E: Sync,
{
	type Error = Infallible;

	async fn from_request(request: Request<B>, _args: &mut Args<'_, E>) -> Result<Self, Self::Error> {
		let (RequestHead { headers, .. }, _) = request.into_parts();

		Ok(headers)
	}
}

// -------------------------

impl IntoResponseHead for HeaderMap {
	type Error = Infallible;

	#[inline]
	fn into_response_head(self, mut head: ResponseHead) -> Result<ResponseHead, Self::Error> {
		head.headers.extend(self);

		Ok(head)
	}
}

impl IntoResponse for HeaderMap {
	fn into_response(self) -> Response {
		let mut response = ().into_response();
		*response.headers_mut() = self;

		response
	}
}

// --------------------------------------------------

#[derive(Debug, ImplError)]
pub(crate) enum ContentTypeError {
	#[error("missing {0} header")]
	MissingHeader(HeaderName),
	#[error(transparent)]
	InvalidValue(#[from] ToStrError),
}

// --------------------------------------------------

pub fn split_header_value(
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
