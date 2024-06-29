use std::{
	any::{Any, TypeId},
	fmt::{self, Display, Formatter},
};

use http::StatusCode;

use crate::{body::Body, marker, BoxedError, StdError, SCOPE_VALIDITY};

use super::{BoxedErrorResponse, IntoResponse, Response};

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

// --------------------------------------------------
// ResponseError

/// An error response type that implements both [`Error`](std::error::Error) and
/// [`IntoResponse`] traits.
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
			f.write_fmt(format_args!(" {}", boxed_error))?
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

/// Blankedly implemented by error types that can be converted into the [`Response`] type.
pub trait ErrorResponse: StdError + IntoResponse + 'static {
	#[doc(hidden)]
	fn concrete_type_id(&self, _: marker::Private) -> TypeId {
		TypeId::of::<Self>()
	}

	#[doc(hidden)]
	fn as_any(self: Box<Self>, _: marker::Private) -> Box<dyn Any>;

	#[doc(hidden)]
	fn as_any_ref(&self, _: marker::Private) -> &dyn Any;

	#[doc(hidden)]
	fn as_any_mut(&mut self, _: marker::Private) -> &mut dyn Any;

	#[doc(hidden)]
	fn concrete_into_response(self: Box<Self>, _: marker::Private) -> Response;

	/// Converts the `ErrorResponse` into `ResponseResult::Err(BoxedErrorResponse)`.
	fn into_error_result(self) -> Result<Response, BoxedErrorResponse>
	where
		Self: Sized + Send + Sync,
	{
		Err(self.into())
	}
}

impl dyn ErrorResponse + Send + Sync {
	/// Returns the [`TypeId`] of the implementor.
	pub fn implementor_type_id(&self) -> TypeId {
		ErrorResponse::concrete_type_id(self, marker::Private)
	}

	/// Checks whether the implementor of the trait is the given type `E`.
	pub fn is<E: Any + 'static>(&self) -> bool {
		let self_id = ErrorResponse::concrete_type_id(self, marker::Private);
		let param_id = TypeId::of::<E>();

		self_id == param_id
	}

	/// Casts the trait object into type `E` if it's an underlying concrete type.
	pub fn downcast_to<E: Any + 'static>(self: Box<Self>) -> Result<Box<E>, Box<Self>> {
		if self.is::<E>() {
			Ok(
				self
					.as_any(marker::Private)
					.downcast()
					.expect(SCOPE_VALIDITY),
			)
		} else {
			Err(self)
		}
	}

	/// Returns a reference to an underlying concrete type if it's a type `E`.
	pub fn downcast_to_ref<E: Any + 'static>(&self) -> Option<&E> {
		self.as_any_ref(marker::Private).downcast_ref()
	}

	/// Returns a mutable reference to an underlying concrete type if it's a type `E`.
	pub fn downcast_to_mut<E: Any + 'static>(&mut self) -> Option<&mut E> {
		self.as_any_mut(marker::Private).downcast_mut()
	}

	/// Converts the error into `Response`.
	pub fn into_response(self: Box<Self>) -> Response {
		self.concrete_into_response(marker::Private)
	}
}

impl<E> ErrorResponse for E
where
	E: StdError + IntoResponse + Send + Sync + 'static,
{
	#[doc(hidden)]
	fn as_any(self: Box<Self>, _: marker::Private) -> Box<dyn Any> {
		self
	}

	#[doc(hidden)]
	fn as_any_ref(&self, _: marker::Private) -> &dyn Any {
		self
	}

	#[doc(hidden)]
	fn as_any_mut(&mut self, _: marker::Private) -> &mut dyn Any {
		self
	}

	#[doc(hidden)]
	fn concrete_into_response(self: Box<Self>, _: marker::Private) -> Response {
		let e = (self as Box<dyn Any>)
			.downcast::<E>()
			.expect(SCOPE_VALIDITY);

		(*e).into_response()
	}
}

impl<E: ErrorResponse + Send + Sync> From<E> for BoxedErrorResponse {
	fn from(error_response: E) -> Self {
		Box::new(error_response)
	}
}

// --------------------------------------------------------------------------------

#[cfg(test)]
mod test {
	use std::fmt::Display;

	use crate::response::IntoResponseResult;

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

		// ----------

		let response_error = ResponseError::from(StatusCode::INTERNAL_SERVER_ERROR);
		let result = response_error.into_error_result();
		assert!(result.is_err());
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
		let mut error = Box::new(E) as BoxedErrorResponse;
		assert_eq!(TypeId::of::<E>(), error.implementor_type_id());
		assert!(error.is::<E>());
		assert_eq!(Some(&mut E), error.downcast_to_mut::<E>());
		assert_eq!(Some(&E), error.downcast_to_ref::<E>());
		assert_eq!(E, error.downcast_to::<E>().map(|boxed| *boxed).unwrap());

		// ----------

		let mut error = Box::new(A) as BoxedErrorResponse;
		assert_eq!(
			TypeId::of::<A>(),
			<dyn ErrorResponse + Send + Sync>::implementor_type_id(error.as_ref())
		);
		assert!(error.is::<A>());
		assert!(!error.is::<E>());
		assert_eq!(Some(&mut A), error.downcast_to_mut::<A>());
		assert_eq!(Some(&A), error.downcast_to_ref::<A>());
		assert_eq!(None, error.downcast_to_mut::<E>());
		assert_eq!(None, error.downcast_to_ref::<E>());

		let result = error.downcast_to::<E>();
		assert!(result.is_err());
		assert_eq!(
			A,
			result
				.unwrap_err()
				.downcast_to::<A>()
				.map(|boxed| *boxed)
				.unwrap()
		);

		// ----------

		let error = Box::new(A) as BoxedErrorResponse;
		assert_eq!(
			A,
			error
				.downcast_to::<E>()
				.unwrap_err()
				.downcast_to::<A>()
				.map(|boxed| *boxed)
				.unwrap()
		);

		// ----------

		let response_result = E.into_error_result();
		assert!(response_result.is_err());

		let response_result = E.into_response_result();
		assert!(response_result.is_ok());
	}
}

// --------------------------------------------------------------------------------
