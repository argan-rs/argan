use super::*;

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

// --------------------------------------------------
// RequestExtensions

pub struct RequestExtension<T>(pub T);

impl<E, T> FromRequestHead<E> for RequestExtension<T>
where
	E: Sync,
	T: Clone + Send + Sync + 'static,
{
	type Error = StatusCode; // TODO.

	async fn from_request_head(
		head: &mut RequestHead,
		_args: &mut Args<'_, E>,
	) -> Result<Self, Self::Error> {
		match head.extensions.get::<T>() {
			Some(value) => Ok(RequestExtension(value.clone())),
			None => Err(StatusCode::INTERNAL_SERVER_ERROR),
		}
	}
}

impl<B, E, T> FromRequest<B, E> for RequestExtension<T>
where
	B: Send,
	E: Sync,
	T: Clone + Send + Sync + 'static,
{
	type Error = StatusCode;

	async fn from_request(request: Request<B>, _args: &mut Args<'_, E>) -> Result<Self, Self::Error> {
		let (mut head, _) = request.into_parts();

		Self::from_request_head(&mut head, _args).await
	}
}

impl<T> IntoResponseHead for RequestExtension<T>
where
	T: Clone + Send + Sync + 'static,
{
	type Error = Infallible;

	#[inline(always)]
	fn into_response_head(self, mut head: ResponseHead) -> Result<ResponseHead, Self::Error> {
		let RequestExtension(value) = self;

		if head.extensions.insert(value).is_some() {
			panic!(
				"type {} has already been used as a response extension",
				any::type_name::<T>()
			);
		}

		Ok(head)
	}
}

impl<T> IntoResponse for RequestExtension<T>
where
	T: Clone + Send + Sync + 'static,
{
	#[inline(always)]
	fn into_response(self) -> Response {
		let RequestExtension(value) = self;

		let mut response = Response::default();
		if response.extensions_mut().insert(value).is_some() {
			panic!(
				"type {} has already been used as a response extension",
				any::type_name::<T>()
			);
		}

		response
	}
}

// --------------------------------------------------
// HandlerExtension

#[derive(Clone)]
pub struct HandlerExtension<E>(E);

impl<E> FromRequestHead<E> for HandlerExtension<E>
where
	E: Clone + Sync,
{
	type Error = Infallible;

	#[inline]
	async fn from_request_head(
		head: &mut RequestHead,
		args: &mut Args<'_, E>,
	) -> Result<Self, Self::Error> {
		Ok(HandlerExtension(args.handler_extension.clone()))
	}
}

impl<B, E> FromRequest<B, E> for HandlerExtension<E>
where
	B: Send,
	E: Clone + Sync,
{
	type Error = Infallible;

	#[inline]
	async fn from_request(request: Request<B>, args: &mut Args<'_, E>) -> Result<Self, Self::Error> {
		Ok(HandlerExtension(args.handler_extension.clone()))
	}
}

// --------------------------------------------------
// ResourceExtension

pub struct ResourceExtension<RE>(RE);

impl<HE, RE> FromRequestHead<HE> for ResourceExtension<RE>
where
	HE: Sync,
	RE: Clone + Send + Sync + 'static,
{
	type Error = StatusCode; // ???

	#[inline]
	async fn from_request_head(
		head: &mut RequestHead,
		args: &mut Args<'_, HE>,
	) -> Result<Self, Self::Error> {
		match args.resource_extensions.get_ref::<RE>() {
			Some(value) => Ok(Self(value.clone())),
			None => Err(StatusCode::INTERNAL_SERVER_ERROR),
		}
	}
}

impl<B, HE, RE> FromRequest<B, HE> for ResourceExtension<RE>
where
	B: Send,
	HE: Sync,
	RE: Clone + Send + Sync + 'static,
{
	type Error = StatusCode; // ???

	#[inline]
	async fn from_request(request: Request<B>, args: &mut Args<'_, HE>) -> Result<Self, Self::Error> {
		match args.resource_extensions.get_ref::<RE>() {
			Some(value) => Ok(Self(value.clone())),
			None => Err(StatusCode::INTERNAL_SERVER_ERROR),
		}
	}
}

// --------------------------------------------------------------------------------
