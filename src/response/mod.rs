//! HTTP response types.

// ----------

use std::{
	any::type_name,
	convert::Infallible,
	fmt::{Debug, Display},
	future::Future,
	marker::PhantomData,
	pin::Pin,
	task::{Context, Poll},
};

use argan_core::body::Body;
use futures_util::FutureExt;
use http::{
	header::{CONTENT_TYPE, LOCATION},
	HeaderValue, StatusCode,
};

// ----------

pub use argan_core::response::*;

// --------------------------------------------------

#[cfg(feature = "sse")]
pub mod event_stream;

#[cfg(feature = "file-stream")]
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
		let mut response = self.status_code.into_response();
		response.headers_mut().insert(LOCATION, self.uri);

		response
	}
}

// --------------------------------------------------
// Html

/// An HTML response that has a `Content-Type: text/html; charset=utf-8`.
#[derive(Debug, Clone)]
pub struct Html<B>(pub B);

impl<B: Into<Body>> IntoResponse for Html<B> {
	fn into_response(self) -> Response {
		let mut response = Response::new(self.0.into());
		response.headers_mut().insert(
			CONTENT_TYPE,
			HeaderValue::from_static(mime::TEXT_HTML_UTF_8.as_ref()),
		);

		response
	}
}

// --------------------------------------------------
// ResponseExtension

/// Adds the given type value to the [`ResponseHeadParts`] extensions.
pub struct ResponseExtension<T>(pub T);

impl<T> IntoResponseHeadParts for ResponseExtension<T>
where
	T: Clone + Send + Sync + 'static,
{
	#[inline]
	fn into_response_head(
		self,
		mut head: ResponseHeadParts,
	) -> Result<ResponseHeadParts, BoxedErrorResponse> {
		let ResponseExtension(value) = self;

		if head.extensions.insert(value).is_some() {
			return Err(ResponseExtensionError::<T>(PhantomData).into());
		}

		Ok(head)
	}
}

impl<T> IntoResponseResult for ResponseExtension<T>
where
	T: Clone + Send + Sync + 'static,
{
	#[inline]
	fn into_response_result(self) -> Result<Response, BoxedErrorResponse> {
		let ResponseExtension(value) = self;

		let mut response = Response::default();
		if response.extensions_mut().insert(value).is_some() {
			return Err(ResponseExtensionError::<T>(PhantomData).into());
		}

		Ok(response)
	}
}

// -------------------------
// ResponseExtensionError

/// An error that's returned when the given type value already exists in the [`ResponseHeadParts`].
pub struct ResponseExtensionError<T>(PhantomData<T>);

impl<T> Debug for ResponseExtensionError<T> {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "ResponseExtensionError<{}>", type_name::<T>())
	}
}

impl<T> Display for ResponseExtensionError<T> {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(
			f,
			"type {0} has already been used as a response extension",
			type_name::<T>()
		)
	}
}

impl<T> crate::StdError for ResponseExtensionError<T> {}

impl<T> IntoResponse for ResponseExtensionError<T> {
	fn into_response(self) -> Response {
		StatusCode::INTERNAL_SERVER_ERROR.into_response()
	}
}

// --------------------------------------------------------------------------------
