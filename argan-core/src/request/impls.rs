use http::{HeaderMap, Method, Uri, Version};

use crate::IntoArray;

use super::*;

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

// --------------------------------------------------
// Result<T, E>

impl<PE, HE, T, E> FromRequestHead<PE, HE> for Result<T, E>
where
	PE: Send,
	HE: Sync,
	T: FromRequestHead<PE, HE, Error = E>,
{
	type Error = Infallible;

	async fn from_request_head(
		head: &mut RequestHead,
		args: &mut Args<'_, PE, HE>,
	) -> Result<Self, Self::Error> {
		let result = T::from_request_head(head, args).await;

		Ok(result)
	}
}

impl<B, PE, HE, T, E> FromRequest<B, PE, HE> for Result<T, E>
where
	B: Send,
	PE: Send,
	HE: Sync,
	T: FromRequest<B, PE, HE, Error = E>,
{
	type Error = Infallible;

	async fn from_request(
		request: Request<B>,
		args: &mut Args<'_, PE, HE>,
	) -> Result<Self, Self::Error> {
		let result = T::from_request(request, args).await;

		Ok(result)
	}
}

// --------------------------------------------------
// Method

impl<PE, HE> FromRequestHead<PE, HE> for Method
where
	PE: Send,
	HE: Sync,
{
	type Error = Infallible;

	async fn from_request_head(
		head: &mut RequestHead,
		_args: &mut Args<'_, PE, HE>,
	) -> Result<Self, Self::Error> {
		Ok(head.method.clone())
	}
}

impl<B, PE, HE> FromRequest<B, PE, HE> for Method
where
	B: Send,
	PE: Send,
	HE: Sync,
{
	type Error = Infallible;

	async fn from_request(
		request: Request<B>,
		_args: &mut Args<'_, PE, HE>,
	) -> Result<Self, Self::Error> {
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

impl<PE, HE> FromRequestHead<PE, HE> for Uri
where
	PE: Send,
	HE: Sync,
{
	type Error = Infallible;

	async fn from_request_head(
		head: &mut RequestHead,
		_args: &mut Args<'_, PE, HE>,
	) -> Result<Self, Self::Error> {
		Ok(head.uri.clone())
	}
}

impl<B, PE, HE> FromRequest<B, PE, HE> for Uri
where
	B: Send,
	PE: Send,
	HE: Sync,
{
	type Error = Infallible;

	async fn from_request(
		request: Request<B>,
		_args: &mut Args<'_, PE, HE>,
	) -> Result<Self, Self::Error> {
		let (head, _) = request.into_parts();

		Ok(head.uri)
	}
}

// --------------------------------------------------
// Version

impl<PE, HE> FromRequestHead<PE, HE> for Version
where
	PE: Send,
	HE: Sync,
{
	type Error = Infallible;

	async fn from_request_head(
		head: &mut RequestHead,
		_args: &mut Args<'_, PE, HE>,
	) -> Result<Self, Self::Error> {
		Ok(head.version)
	}
}

impl<B, PE, HE> FromRequest<B, PE, HE> for Version
where
	B: Send,
	PE: Send,
	HE: Sync,
{
	type Error = Infallible;

	async fn from_request(
		request: Request<B>,
		_args: &mut Args<'_, PE, HE>,
	) -> Result<Self, Self::Error> {
		let (head, _) = request.into_parts();

		Ok(head.version)
	}
}

// --------------------------------------------------
// HeaderMap

impl<PE, HE> FromRequestHead<PE, HE> for HeaderMap
where
	PE: Send,
	HE: Sync,
{
	type Error = Infallible;

	async fn from_request_head(
		head: &mut RequestHead,
		_args: &mut Args<'_, PE, HE>,
	) -> Result<Self, Self::Error> {
		Ok(head.headers.clone())
	}
}

impl<B, PE, HE> FromRequest<B, PE, HE> for HeaderMap
where
	B: Send,
	PE: Send,
	HE: Sync,
{
	type Error = Infallible;

	async fn from_request(
		request: Request<B>,
		_args: &mut Args<'_, PE, HE>,
	) -> Result<Self, Self::Error> {
		let (RequestHead { headers, .. }, _) = request.into_parts();

		Ok(headers)
	}
}

// --------------------------------------------------
// Tuples

macro_rules! impl_extractions_for_tuples {
	($t1:ident, $(($($t:ident),*),)? $tl:ident) => {
		#[allow(non_snake_case)]
		impl<$t1, $($($t,)*)? $tl, PE, HE>
		FromRequestHead<PE, HE> for ($t1, $($($t,)*)? $tl)
		where
			$t1: FromRequestHead<PE, HE> + Send,
			$($($t: FromRequestHead<PE, HE> + Send,)*)?
			$tl: FromRequestHead<PE, HE> + Send,
			PE: Send,
			HE: Sync,
		{
			type Error = BoxedErrorResponse;

			async fn from_request_head(
				head: &mut RequestHead,
				args: &mut Args<'_, PE, HE>,
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
		impl<$t1, $($($t,)*)? $tl, B, PE, HE>
		FromRequest<B, PE, HE> for ($t1, $($($t,)*)? $tl)
		where
			$t1: FromRequestHead<PE, HE> + Send,
			$($($t: FromRequestHead<PE, HE> + Send,)*)?
			$tl: FromRequest<B, PE, HE> + Send,
			B: Send,
			PE: Send,
			HE: Sync,
		{
			type Error = BoxedErrorResponse;

			async fn from_request(
				request: Request<B>,
				args: &mut Args<'_, PE, HE>,
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
