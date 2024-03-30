use futures_util::FutureExt;
use http::{HeaderMap, Method, Uri, Version};

use crate::IntoArray;

use super::*;

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

// --------------------------------------------------
// Result<T, E>

impl<Args, T, E> FromRequestHead<Args> for Result<T, E>
where
	T: FromRequestHead<Args, Error = E>,
{
	type Error = Infallible;

	fn from_request_head(
		head: &mut RequestHead,
		args: &Args,
	) -> impl Future<Output = Result<Self, Self::Error>> {
		T::from_request_head(head, args).map(|result| Ok(result))
	}
}

impl<'r, B, Args, T, E: 'r> FromRequestRef<'r, B, Args> for Result<T, E>
where
	T: FromRequestRef<'r, B, Args, Error = E>,
{
	type Error = Infallible;

	#[inline(always)]
	fn from_request_ref(
		request: &'r Request<B>,
		args: Option<&'r Args>,
	) -> impl Future<Output = Result<Self, Self::Error>> {
		T::from_request_ref(request, args).map(|result| Ok(result))
	}
}

impl<B, Args, T, E> FromRequest<B, Args> for Result<T, E>
where
	T: FromRequest<B, Args, Error = E>,
{
	type Error = Infallible;

	fn from_request(
		request: Request<B>,
		args: Args,
	) -> impl Future<Output = Result<Self, Self::Error>> {
		T::from_request(request, args).map(|result| Ok(result))
	}
}

// --------------------------------------------------
// Method

impl<Args> FromRequestHead<Args> for Method {
	type Error = Infallible;

	fn from_request_head(
		head: &mut RequestHead,
		_args: &Args,
	) -> impl Future<Output = Result<Self, Self::Error>> {
		ready(Ok(head.method.clone()))
	}
}

impl<B, Args> FromRequest<B, Args> for Method {
	type Error = Infallible;

	fn from_request(
		request: Request<B>,
		_args: Args,
	) -> impl Future<Output = Result<Self, Self::Error>> {
		let (head, _) = request.into_parts();

		ready(Ok(head.method))
	}
}

// ----------

impl IntoArray<Method, 1> for Method {
	fn into_array(self) -> [Method; 1] {
		[self]
	}
}

// --------------------------------------------------
// Uri

impl<Args> FromRequestHead<Args> for Uri {
	type Error = Infallible;

	fn from_request_head(
		head: &mut RequestHead,
		_args: &Args,
	) -> impl Future<Output = Result<Self, Self::Error>> {
		ready(Ok(head.uri.clone()))
	}
}

impl<B, Args> FromRequest<B, Args> for Uri {
	type Error = Infallible;

	fn from_request(
		request: Request<B>,
		_args: Args,
	) -> impl Future<Output = Result<Self, Self::Error>> {
		let (head, _) = request.into_parts();

		ready(Ok(head.uri))
	}
}

// --------------------------------------------------
// Version

impl<Args> FromRequestHead<Args> for Version {
	type Error = Infallible;

	fn from_request_head(
		head: &mut RequestHead,
		_args: &Args,
	) -> impl Future<Output = Result<Self, Self::Error>> {
		ready(Ok(head.version))
	}
}

impl<B, Args> FromRequest<B, Args> for Version {
	type Error = Infallible;

	fn from_request(
		request: Request<B>,
		_args: Args,
	) -> impl Future<Output = Result<Self, Self::Error>> {
		let (head, _) = request.into_parts();

		ready(Ok(head.version))
	}
}

// --------------------------------------------------
// HeaderMap

impl<Args> FromRequestHead<Args> for HeaderMap {
	type Error = Infallible;

	fn from_request_head(
		head: &mut RequestHead,
		_args: &Args,
	) -> impl Future<Output = Result<Self, Self::Error>> {
		ready(Ok(head.headers.clone()))
	}
}

impl<B, Args> FromRequest<B, Args> for HeaderMap {
	type Error = Infallible;

	fn from_request(
		request: Request<B>,
		_args: Args,
	) -> impl Future<Output = Result<Self, Self::Error>> {
		let (RequestHead { headers, .. }, _) = request.into_parts();

		ready(Ok(headers))
	}
}

// --------------------------------------------------
// Tuples

macro_rules! impl_extractions_for_tuples {
	($t1:ident, $(($($t:ident),*),)? $tl:ident) => {
		#[allow(non_snake_case)]
		impl<$t1, $($($t,)*)? $tl, Args> FromRequestHead<Args> for ($t1, $($($t,)*)? $tl)
		where
			$t1: FromRequestHead<Args> + Send,
			$($($t: FromRequestHead<Args> + Send,)*)?
			$tl: FromRequestHead<Args> + Send,
			Args: Sync,
		{
			type Error = BoxedErrorResponse;

			async fn from_request_head(
				head: &mut RequestHead,
				args: &Args,
			) -> Result<Self, Self::Error> {
				let $t1 = $t1::from_request_head(head, args).await.map_err(Into::into)?;

				$(
					$(
						let $t = $t::from_request_head(head, args).await.map_err(Into::into)?;
					)*
				)?

				let $tl = $tl::from_request_head(head, args).await.map_err(Into::into)?;

				Ok(($t1, $($($t,)*)? $tl))
			}
		}

		#[allow(non_snake_case)]
		impl<$t1, $($($t,)*)? $tl, B, Args>
		FromRequest<B, Args> for ($t1, $($($t,)*)? $tl)
		where
			$t1: FromRequestHead<Args> + Send,
			$($($t: FromRequestHead<Args> + Send,)*)?
			$tl: FromRequest<B, Args> + Send,
			B: Send,
			Args: Send,
		{
			type Error = BoxedErrorResponse;

			async fn from_request(
				request: Request<B>,
				args: Args,
			) -> Result<Self, Self::Error> {
				let (mut head, body) = request.into_parts();

				let $t1 = $t1::from_request_head(&mut head, &args).await.map_err(Into::into)?;

				$($(
					let $t = $t::from_request_head(&mut head, &args).await.map_err(Into::into)?;
				)*)?

				let request = Request::from_parts(head, body);

				let $tl = $tl::from_request(request, args).await.map_err(Into::into)?;

				Ok(($t1, $($($t,)*)? $tl))
			}
		}
	};
}

call_for_tuples!(impl_extractions_for_tuples!);

// --------------------------------------------------------------------------------
