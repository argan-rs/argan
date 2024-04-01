use super::*;

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

// --------------------------------------------------
// SizeLimit

pub enum SizeLimit {
	Default,
	Value(usize),
}

// --------------------------------------------------
// PathParams

pub struct PathParams<T>(pub T);

impl<'de, T> PathParams<T>
where
	T: Deserialize<'de>,
{
	pub(crate) fn deserialize<D: Deserializer<'de>>(
		&mut self,
		deserializer: D,
	) -> Result<(), D::Error> {
		self.0 = T::deserialize(deserializer)?;

		Ok(())
	}
}

// impl<T> FromMutRequestHead for PathParams<T>
// where
// 	T: DeserializeOwned,
// {
// 	type Error = PathParamsError;
//
// 	async fn from_request_head(
// 		head: &mut RequestHead,
// 	) -> Result<Self, Self::Error> {
// 		let routing_state = head.extensions.get::<Uncloneable<RoutingState>>()
// 			.expect("Uncloneable<RoutingState> should always be present in request extensions")
// 			.as_ref()
// 			.expect("RoutingState should always be present in Uncloneable");
//
// 		let mut from_params_list = routing_state.uri_params.deserializer();
//
// 		ready(
// 			T::deserialize(&mut from_params_list)
// 				.map(|value| Self(value))
// 				.map_err(Into::into),
// 		)
// 	}
// }

// impl<'r, B, T> FromRequestRef<'r, B> for PathParams<T>
// where
// 	B: Sync,
// 	T: Deserialize<'r> + Send + 'r,
// {
// 	type Error = PathParamsError;
//
// 	async fn from_request_ref(request: &'r Request<B>) -> Result<Self, Self::Error> {
// 		let mut from_params_list = request.uri_params_deserializer();
//
// 		T::deserialize(&mut from_params_list)
// 			.map(|value| Self(value))
// 			.map_err(Into::into)
// 	}
// }
//
// impl<B, T> FromRequest<B> for PathParams<T>
// where
// 	B: Send,
// 	T: DeserializeOwned,
// {
// 	type Error = PathParamsError;
//
// 	async fn from_request(request: Request<B>) -> Result<Self, Self::Error> {
// 		let mut from_params_list = request.uri_params_deserializer();
//
// 		T::deserialize(&mut from_params_list)
// 			.map(|value| Self(value))
// 			.map_err(Into::into)
// 	}
// }

impl<T> Debug for PathParams<T>
where
	T: Debug,
{
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_tuple("PathParams").field(&self.0).finish()
	}
}

// ----------

#[derive(Debug, crate::ImplError)]
#[error(transparent)]
pub struct PathParamsError(#[from] pub(crate) pattern::DeserializerError);

impl IntoResponse for PathParamsError {
	fn into_response(self) -> Response {
		match self.0 {
			pattern::DeserializerError::ParsingFailue(_) => StatusCode::NOT_FOUND.into_response(),
			_ => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
		}
	}
}

// --------------------------------------------------
// QueryParams

pub struct QueryParams<T>(pub T);

// impl<T> FromMutRequestHead for QueryParams<T>
// where
// 	T: DeserializeOwned,
// {
// 	type Error = QueryParamsError;
//
// 	async fn from_request_head(head: &mut RequestHead) -> Result<Self, Self::Error> {
// 		let query_string = head
// 			.uri
// 			.query()
// 			.ok_or(QueryParamsError(QueryParamsErrorValue::NoDataIsAvailable))?;
//
// 		serde_urlencoded::from_str::<T>(query_string)
// 			.map(|value| Self(value))
// 			.map_err(|error| QueryParamsError(error.into()))
// 	}
// }

impl<'r, B, T> FromRequestRef<'r, B> for QueryParams<T>
where
	B: Sync,
	T: Deserialize<'r> + Send + 'r,
{
	type Error = QueryParamsError;

	async fn from_request_ref(request: &'r Request<B>) -> Result<Self, Self::Error> {
		let query_string = request
			.uri()
			.query()
			.ok_or(QueryParamsError::NoDataIsAvailable)?;

		serde_urlencoded::from_str::<T>(query_string)
			.map(|value| Self(value))
			.map_err(|error| QueryParamsError::InvalidData(error.into()))
	}
}

impl<B, T> FromRequest<B> for QueryParams<T>
where
	B: Send,
	T: DeserializeOwned,
{
	type Error = QueryParamsError;

	async fn from_request(request: Request<B>) -> Result<Self, Self::Error> {
		let query_string = request
			.uri()
			.query()
			.ok_or(QueryParamsError::NoDataIsAvailable)?;

		serde_urlencoded::from_str::<T>(query_string)
			.map(|value| Self(value))
			.map_err(|error| QueryParamsError::InvalidData(error.into()))
	}
}

#[derive(Debug, crate::ImplError)]
pub enum QueryParamsError {
	#[error("no data is available")]
	NoDataIsAvailable,
	#[error(transparent)]
	InvalidData(#[from] serde_urlencoded::de::Error),
}

impl IntoResponse for QueryParamsError {
	fn into_response(self) -> Response {
		StatusCode::BAD_REQUEST.into_response()
	}
}

// --------------------------------------------------
// RemainingPath

pub struct RemainingPath<'r>(pub Cow<'r, str>);

// impl FromMutRequestHead for RemainingPath {
// 	type Error = Infallible;
//
// 	async fn from_request_head(head: &mut RequestHead) -> Result<Self, Self::Error> {
// 		let routing_state = head.extensions.get::<Uncloneable<RoutingState>>()
// 			.expect("Uncloneable<RoutingState> should always be present in request extensions")
// 			.as_ref()
// 			.expect("RoutingState should always be present in Uncloneable");
//
//		routing_state
//			.route_traversal
//			.remaining_segments(head.uri.path())
//			.map_or(Ok(RemainingPath::None), |remaining_path| {
//				Ok(RemainingPath::Owned(remaining_path.into()))
//			}),
// 	}
// }

// impl<'r, B> FromRequestRef<'r, B> for RemainingPath<'r>
// where
// 	B: Sync,
// {
// 	type Error = Infallible;
//
// 	async fn from_request_ref(request: &'r Request<B>) -> Result<Self, Self::Error> {
// 		Ok(Self(request.routing_remaining_segments().into()))
// 	}
// }
//
// impl<B> FromRequest<B> for RemainingPath<'static>
// where
// 	B: Send + 'static,
// {
// 	type Error = Infallible;
//
// 	async fn from_request(request: Request<B>) -> Result<Self, Self::Error> {
// 		Ok(Self(Cow::Owned(
// 			request.routing_remaining_segments().into(),
// 		)))
// 	}
// }

// --------------------------------------------------------------------------------
