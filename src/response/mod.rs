use std::{
	any::Any,
	convert::Infallible,
	fmt::Display,
	future::Future,
	pin::Pin,
	task::{Context, Poll},
};

use futures_util::FutureExt;
use http::{
	header::{InvalidHeaderName, InvalidHeaderValue, LOCATION},
	response::Parts,
	HeaderMap, HeaderName, HeaderValue, StatusCode,
};

// ----------

pub use argan_core::response::*;

// --------------------------------------------------

pub mod event_stream;
pub mod file_stream;

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

// --------------------------------------------------
// InffallibelResponseFuture

mod private {
	use argan_core::BoxedFuture;

	use super::*;

	pub struct InfallibleResponseFuture(BoxedFuture<Result<Response, BoxedErrorResponse>>);

	impl InfallibleResponseFuture {
		pub(crate) fn from(boxed_future: BoxedFuture<Result<Response, BoxedErrorResponse>>) -> Self {
			Self(boxed_future)
		}
	}

	impl Future for InfallibleResponseFuture {
		type Output = Result<Response, Infallible>;

		#[inline]
		fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
			self.0.poll_unpin(cx).map(|output| match output {
				Ok(response) => Ok(response),
				Err(error_response) => Ok(error_response.into_response()),
			})
		}
	}
}

pub(crate) use private::InfallibleResponseFuture;

// --------------------------------------------------
// Redirect

/// Returned by handlers to redirect requests.
pub struct Redirect {
	status_code: StatusCode,
	uri: HeaderValue,
}

impl Redirect {
	/// 308 Permanent Redirect
	pub fn permanently_to<U: AsRef<str>>(uri: U) -> Self {
		Self {
			status_code: StatusCode::PERMANENT_REDIRECT,
			uri: HeaderValue::from_str(uri.as_ref()).expect("uri must be a valid header value"),
		}
	}

	/// 307 Temporary Redirect
	pub fn temporarily_to<U: AsRef<str>>(uri: U) -> Self {
		Self {
			status_code: StatusCode::TEMPORARY_REDIRECT,
			uri: HeaderValue::from_str(uri.as_ref()).expect("uri must be a valid header value"),
		}
	}

	/// 303 See Other
	pub fn to_see<U: AsRef<str>>(uri: U) -> Self {
		Self {
			status_code: StatusCode::SEE_OTHER,
			uri: HeaderValue::from_str(uri.as_ref()).expect("uri must be a valid header value"),
		}
	}
}

impl IntoResponse for Redirect {
	#[inline]
	fn into_response(self) -> Response {
		let mut response = Response::default();
		*response.status_mut() = self.status_code;
		response.headers_mut().insert(LOCATION, self.uri);

		response
	}
}

// --------------------------------------------------------------------------------
