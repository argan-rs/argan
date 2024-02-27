use std::{
	any::{Any, TypeId},
	fmt::{self, Display, Formatter, Write},
};

use bytes::Bytes;
use http::StatusCode;

use crate::{
	body::Body,
	common::{mark, BoxedError, SCOPE_VALIDITY},
	StdError,
};

use super::{BoxedErrorResponse, IntoResponse, Response};

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

// --------------------------------------------------
// ResponseError

#[derive(Debug)]
pub struct ResponseError {
	status_code: StatusCode,
	some_boxed_error: Option<BoxedError>,
}

impl ResponseError {
	pub fn new<E>(status_code: StatusCode, error: E) -> Self
	where
		E: StdError + Send + Sync + 'static,
	{
		ResponseError {
			status_code,
			some_boxed_error: Some(error.into()),
		}
	}

	pub fn from_error<E>(error: E) -> Self
	where
		E: StdError + Send + Sync + 'static,
	{
		ResponseError {
			status_code: StatusCode::INTERNAL_SERVER_ERROR,
			some_boxed_error: Some(error.into()),
		}
	}

	pub fn status_code(&self) -> StatusCode {
		self.status_code
	}
}

impl From<StatusCode> for ResponseError {
	fn from(status_code: StatusCode) -> Self {
		ResponseError {
			status_code,
			some_boxed_error: None,
		}
	}
}

impl Display for ResponseError {
	fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
		f.write_fmt(format_args!("[{}]", self.status_code))?;

		if let Some(boxed_error) = self.some_boxed_error.as_ref() {
			f.write_fmt(format_args!(" {}", boxed_error.to_string()))?
		}

		Ok(())
	}
}

impl StdError for ResponseError {
	fn source(&self) -> Option<&(dyn StdError + 'static)> {
		self
			.some_boxed_error
			.as_ref()
			.map(|boxed_error| boxed_error.as_ref() as &(dyn StdError + 'static))
	}
}

impl IntoResponse for ResponseError {
	fn into_response(self) -> Response {
		let mut response = self.status_code.into_response();

		if let Some(boxed_error) = self.some_boxed_error {
			*response.body_mut() = Body::new(boxed_error.to_string())
		}

		response
	}
}

// --------------------------------------------------
// ErrorResponse

pub trait ErrorResponse: StdError + IntoResponse + 'static {
	#[doc(hidden)]
	fn concrete_type_id(&self, _: mark::Private) -> TypeId {
		TypeId::of::<Self>()
	}

	#[doc(hidden)]
	fn as_any(self: Box<Self>, _: mark::Private) -> Box<dyn Any>;

	#[doc(hidden)]
	fn as_any_ref(&self, _: mark::Private) -> &dyn Any;

	#[doc(hidden)]
	fn as_any_mut(&mut self, _: mark::Private) -> &mut dyn Any;
}

impl dyn ErrorResponse + 'static {
	pub fn implementor_type_id(&self) -> TypeId {
		ErrorResponse::concrete_type_id(self, mark::Private)
	}

	pub fn is<E: Any + 'static>(&self) -> bool {
		let self_id = ErrorResponse::concrete_type_id(self, mark::Private);
		let param_id = TypeId::of::<E>();

		self_id == param_id
	}

	pub fn downcast<E: Any + 'static>(self: Box<Self>) -> Result<Box<E>, Box<Self>> {
		if self.is::<E>() {
			Ok(self.as_any(mark::Private).downcast().expect(SCOPE_VALIDITY))
		} else {
			Err(self)
		}
	}

	pub fn downcast_ref<E: Any + 'static>(&self) -> Option<&E> {
		self.as_any_ref(mark::Private).downcast_ref()
	}

	pub fn downcast_mut<E: Any + 'static>(&mut self) -> Option<&mut E> {
		self.as_any_mut(mark::Private).downcast_mut()
	}
}

impl<E> ErrorResponse for E
where
	E: StdError + IntoResponse + 'static,
{
	#[doc(hidden)]
	fn as_any(self: Box<Self>, _: mark::Private) -> Box<dyn Any> {
		self
	}

	#[doc(hidden)]
	fn as_any_ref(&self, _: mark::Private) -> &dyn Any {
		self
	}

	#[doc(hidden)]
	fn as_any_mut(&mut self, _: mark::Private) -> &mut dyn Any {
		self
	}
}

impl<E: ErrorResponse> From<E> for BoxedErrorResponse {
	fn from(error_response: E) -> Self {
		Box::new(error_response)
	}
}

// --------------------------------------------------------------------------------

#[cfg(test)]
mod test {
	use std::fmt::Display;

	use super::*;

	// --------------------------------------------------------------------------------
	// --------------------------------------------------------------------------------

	#[derive(Debug)]
	struct Failure;

	impl Display for Failure {
		fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
			write!(f, "failure")
		}
	}

	impl StdError for Failure {}

	// ----------

	#[test]
	fn response_error() {
		let response_error = ResponseError::from(StatusCode::BAD_REQUEST);
		println!("{}", response_error);
		assert_eq!("[400 Bad Request]", response_error.to_string());

		let response_error = ResponseError::new(StatusCode::UNAUTHORIZED, Failure);
		println!("{}", response_error);
		assert_eq!("[401 Unauthorized] failure", response_error.to_string());

		let response_error = ResponseError::from_error(Failure);
		println!("{}", response_error);
		assert_eq!(
			"[500 Internal Server Error] failure",
			response_error.to_string()
		);

		let response_error = ResponseError::from_error(Failure);
		assert!(response_error
			.source()
			.is_some_and(|error| error.is::<Failure>()));

		let boxed_error_response = Box::new(response_error) as BoxedErrorResponse;
		assert!(boxed_error_response
			.source()
			.is_some_and(|error| error.is::<Failure>()));
	}

	// --------------------------------------------------------------------------------

	#[derive(Debug, PartialEq)]
	struct E;

	impl Display for E {
		fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
			write!(f, "E")
		}
	}

	impl StdError for E {}

	impl IntoResponse for E {
		fn into_response(self) -> Response {
			().into_response()
		}
	}

	// ----------

	#[derive(Debug, PartialEq)]
	struct A;

	impl Display for A {
		fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
			write!(f, "A")
		}
	}

	impl StdError for A {}

	impl IntoResponse for A {
		fn into_response(self) -> Response {
			().into_response()
		}
	}

	// --------------------------------------------------

	#[test]
	fn error_response() {
		let mut error = Box::new(E) as Box<dyn ErrorResponse>;
		assert_eq!(TypeId::of::<E>(), error.implementor_type_id());
		assert!(error.is::<E>());
		assert_eq!(Some(&mut E), error.downcast_mut::<E>());
		assert_eq!(Some(&E), error.downcast_ref::<E>());
		assert_eq!(E, error.downcast::<E>().map(|boxed| *boxed).unwrap());

		// ----------

		let mut error = Box::new(A) as Box<dyn ErrorResponse>;
		assert_eq!(
			TypeId::of::<A>(),
			<dyn ErrorResponse>::implementor_type_id(error.as_ref())
		);
		assert!(error.is::<A>());
		assert!(!error.is::<E>());
		assert_eq!(Some(&mut A), error.downcast_mut::<A>());
		assert_eq!(Some(&A), error.downcast_ref::<A>());
		assert_eq!(None, error.downcast_mut::<E>());
		assert_eq!(None, error.downcast_ref::<E>());

		let result = error.downcast::<E>();
		assert!(result.is_err());
		assert_eq!(
			A,
			result
				.unwrap_err()
				.downcast::<A>()
				.map(|boxed| *boxed)
				.unwrap()
		);

		// ----------

		let mut error = Box::new(A) as Box<dyn ErrorResponse>;
		assert_eq!(
			A,
			error
				.downcast::<E>()
				.unwrap_err()
				.downcast::<A>()
				.map(|boxed| *boxed)
				.unwrap()
		);
	}
}
