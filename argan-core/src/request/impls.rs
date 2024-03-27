use http::{HeaderMap, Method, Uri, Version};

use crate::IntoArray;

use super::*;

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

// --------------------------------------------------
// Result<T, E>

impl<Args, Ext, T, E> FromRequestHead<Args, Ext> for Result<T, E>
where
	Args: for<'n> Arguments<'n, Ext> + Send,
	Ext: Sync,
	T: FromRequestHead<Args, Ext, Error = E>,
{
	type Error = Infallible;

	async fn from_request_head(head: &mut RequestHead, args: &mut Args) -> Result<Self, Self::Error> {
		let result = T::from_request_head(head, args).await;

		Ok(result)
	}
}

impl<B, Args, Ext, T, E> FromRequest<B, Args, Ext> for Result<T, E>
where
	B: Send,
	Args: for<'n> Arguments<'n, Ext> + Send,
	Ext: Sync,
	T: FromRequest<B, Args, Ext, Error = E>,
{
	type Error = Infallible;

	async fn from_request(request: Request<B>, args: &mut Args) -> Result<Self, Self::Error> {
		let result = T::from_request(request, args).await;

		Ok(result)
	}
}

// --------------------------------------------------
// Method

impl<Args, Ext> FromRequestHead<Args, Ext> for Method
where
	Args: for<'n> Arguments<'n, Ext> + Send,
	Ext: Sync,
{
	type Error = Infallible;

	async fn from_request_head(
		head: &mut RequestHead,
		_args: &mut Args,
	) -> Result<Self, Self::Error> {
		Ok(head.method.clone())
	}
}

impl<B, Args, Ext> FromRequest<B, Args, Ext> for Method
where
	B: Send,
	Args: for<'n> Arguments<'n, Ext> + Send,
	Ext: Sync,
{
	type Error = Infallible;

	async fn from_request(request: Request<B>, _args: &mut Args) -> Result<Self, Self::Error> {
		let (head, _) = request.into_parts();

		Ok(head.method)
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

impl<Args, Ext> FromRequestHead<Args, Ext> for Uri
where
	Args: for<'n> Arguments<'n, Ext> + Send,
	Ext: Sync,
{
	type Error = Infallible;

	async fn from_request_head(
		head: &mut RequestHead,
		_args: &mut Args,
	) -> Result<Self, Self::Error> {
		Ok(head.uri.clone())
	}
}

impl<B, Args, Ext> FromRequest<B, Args, Ext> for Uri
where
	B: Send,
	Args: for<'n> Arguments<'n, Ext> + Send,
	Ext: Sync,
{
	type Error = Infallible;

	async fn from_request(request: Request<B>, _args: &mut Args) -> Result<Self, Self::Error> {
		let (head, _) = request.into_parts();

		Ok(head.uri)
	}
}

// --------------------------------------------------
// Version

impl<Args, Ext> FromRequestHead<Args, Ext> for Version
where
	Args: for<'n> Arguments<'n, Ext> + Send,
	Ext: Sync,
{
	type Error = Infallible;

	async fn from_request_head(
		head: &mut RequestHead,
		_args: &mut Args,
	) -> Result<Self, Self::Error> {
		Ok(head.version)
	}
}

impl<B, Args, Ext> FromRequest<B, Args, Ext> for Version
where
	B: Send,
	Args: for<'n> Arguments<'n, Ext> + Send,
	Ext: Sync,
{
	type Error = Infallible;

	async fn from_request(request: Request<B>, _args: &mut Args) -> Result<Self, Self::Error> {
		let (head, _) = request.into_parts();

		Ok(head.version)
	}
}

// --------------------------------------------------
// HeaderMap

impl<Args, Ext: Sync> FromRequestHead<Args, Ext> for HeaderMap
where
	Args: for<'n> Arguments<'n, Ext> + Send,
	Ext: Sync,
{
	type Error = Infallible;

	async fn from_request_head(
		head: &mut RequestHead,
		_args: &mut Args,
	) -> Result<Self, Self::Error> {
		Ok(head.headers.clone())
	}
}

impl<B, Args, Ext> FromRequest<B, Args, Ext> for HeaderMap
where
	B: Send,
	Args: for<'n> Arguments<'n, Ext> + Send,
	Ext: Sync,
{
	type Error = Infallible;

	async fn from_request(request: Request<B>, _args: &mut Args) -> Result<Self, Self::Error> {
		let (RequestHead { headers, .. }, _) = request.into_parts();

		Ok(headers)
	}
}

// --------------------------------------------------
// Tuples

macro_rules! impl_extractions_for_tuples {
	($t1:ident, $(($($t:ident),*),)? $tl:ident) => {
		#[allow(non_snake_case)]
		impl<$t1, $($($t,)*)? $tl, Args, Ext> FromRequestHead<Args, Ext> for ($t1, $($($t,)*)? $tl)
		where
			$t1: FromRequestHead<Args, Ext> + Send,
			$($($t: FromRequestHead<Args, Ext> + Send,)*)?
			$tl: FromRequestHead<Args, Ext> + Send,
			Args: for <'n> Arguments<'n, Ext> + Send,
			Ext: Sync,
		{
			type Error = BoxedErrorResponse;

			async fn from_request_head(
				head: &mut RequestHead,
				args: &mut Args,
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
		impl<$t1, $($($t,)*)? $tl, B, Args, Ext> FromRequest<B, Args, Ext> for ($t1, $($($t,)*)? $tl)
		where
			$t1: FromRequestHead<Args, Ext> + Send,
			$($($t: FromRequestHead<Args, Ext> + Send,)*)?
			$tl: FromRequest<B, Args, Ext> + Send,
			B: Send,
			Args: for <'n> Arguments<'n, Ext> + Send,
			Ext: Sync,
		{
			type Error = BoxedErrorResponse;

			async fn from_request(
				request: Request<B>,
				args: &mut Args,
			) -> Result<Self, Self::Error> {
				let (mut head, body) = request.into_parts();

				let $t1 = $t1::from_request_head(&mut head, args).await.map_err(Into::into)?;

				$($(
					let $t = $t::from_request_head(&mut head, args).await.map_err(Into::into)?;
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
