use std::convert::Infallible;

use futures_util::FutureExt;
use http::Method;

use crate::IntoArray;

use super::*;

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

// --------------------------------------------------
// Result<T, E>

impl<'r, B, T, E: 'r> FromRequestRef<'r, B> for Result<T, E>
where
	T: FromRequestRef<'r, B, Error = E>,
{
	type Error = Infallible;

	#[inline(always)]
	fn from_request_ref(request: &'r Request<B>) -> impl Future<Output = Result<Self, Self::Error>> {
		T::from_request_ref(request).map(|result| Ok(result))
	}
}

impl<B, T, E> FromRequest<B> for Result<T, E>
where
	T: FromRequest<B, Error = E>,
{
	type Error = Infallible;

	fn from_request(
		head_parts: RequestHeadParts,
		body: B,
	) -> impl Future<Output = (RequestHeadParts, Result<Self, Self::Error>)> {
		T::from_request(head_parts, body).map(|(head, result)| (head, Ok(result)))
	}
}

impl<B, T, E> FromRequest<B> for Option<T>
where
	T: FromRequest<B, Error = E>,
{
	type Error = Infallible;

	fn from_request(
		head_parts: RequestHeadParts,
		body: B,
	) -> impl Future<Output = (RequestHeadParts, Result<Self, Self::Error>)> {
		T::from_request(head_parts, body).map(|(head, result)| (head, Ok(result.ok())))
	}
}

impl<B> FromRequest<B> for () {
	type Error = Infallible;

	fn from_request(
		head_parts: RequestHeadParts,
		_: B,
	) -> impl Future<Output = (RequestHeadParts, Result<Self, Self::Error>)> + Send {
		ready((head_parts, Ok(())))
	}
}

// --------------------------------------------------
// Method

// impl FromMutRequestHead for Method {
// 	type Error = Infallible;
//
// 	#[inline(always)]
// 	fn from_request_head(
// 		head: &mut RequestHead,
// 	) -> impl Future<Output = Result<Self, Self::Error>> {
// 		ready(Ok(head.method.clone()))
// 	}
// }
//
// impl<'r, B> FromRequestRef<'r, B> for &'r Method {
// 	type Error = Infallible;
//
// 	#[inline(always)]
// 	fn from_request_ref(request: &'r Request<B>) -> impl Future<Output = Result<Self, Self::Error>> {
// 		ready(Ok(request.method()))
// 	}
// }
//
// impl<B> FromRequest<B> for Method {
// 	type Error = Infallible;
//
// 	fn from_request(request: Request<B>) -> impl Future<Output = Result<Self, Self::Error>> {
// 		let (head, _) = request.into_parts();
//
// 		ready(Ok(head.method))
// 	}
// }

// ----------

impl IntoArray<Method, 1> for Method {
	fn into_array(self) -> [Method; 1] {
		[self]
	}
}

// --------------------------------------------------
// Uri

// impl FromMutRequestHead for Uri {
// 	type Error = Infallible;
//
// 	fn from_request_head(
// 		head: &mut RequestHead,
// 	) -> impl Future<Output = Result<Self, Self::Error>> {
// 		ready(Ok(head.uri.clone()))
// 	}
// }

// impl<'r, B> FromRequestRef<'r, B> for &'r Uri {
// 	type Error = Infallible;
//
// 	#[inline(always)]
// 	fn from_request_ref(request: &'r Request<B>) -> impl Future<Output = Result<Self, Self::Error>> {
// 		ready(Ok(request.uri()))
// 	}
// }
//
// impl<B> FromRequest<B> for Uri {
// 	type Error = Infallible;
//
// 	fn from_request(request: Request<B>) -> impl Future<Output = Result<Self, Self::Error>> {
// 		let (head, _) = request.into_parts();
//
// 		ready(Ok(head.uri))
// 	}
// }

// --------------------------------------------------
// Version

// impl FromMutRequestHead for Version {
// 	type Error = Infallible;
//
// 	fn from_request_head(
// 		head: &mut RequestHead,
// 	) -> impl Future<Output = Result<Self, Self::Error>> {
// 		ready(Ok(head.version))
// 	}
// }

// impl<B> FromRequest<B> for Version {
// 	type Error = Infallible;
//
// 	fn from_request(request: Request<B>) -> impl Future<Output = Result<Self, Self::Error>> {
// 		let (head, _) = request.into_parts();
//
// 		ready(Ok(head.version))
// 	}
// }

// --------------------------------------------------
// HeaderMap

// impl FromMutRequestHead for HeaderMap {
// 	type Error = Infallible;
//
// 	fn from_request_head(
// 		head: &mut RequestHead,
// 	) -> impl Future<Output = Result<Self, Self::Error>> {
// 		ready(Ok(head.headers.clone()))
// 	}
// }

// impl<'r, B> FromRequestRef<'r, B> for &'r HeaderMap {
// 	type Error = Infallible;
//
// 	#[inline(always)]
// 	fn from_request_ref(request: &'r Request<B>) -> impl Future<Output = Result<Self, Self::Error>> {
// 		ready(Ok(request.headers()))
// 	}
// }
//
// impl<B> FromRequest<B> for HeaderMap {
// 	type Error = Infallible;
//
// 	fn from_request(request: Request<B>) -> impl Future<Output = Result<Self, Self::Error>> {
// 		let (RequestHead { headers, .. }, _) = request.into_parts();
//
// 		ready(Ok(headers))
// 	}
// }

// --------------------------------------------------
// Tuples

// macro_rules! impl_extractions_for_tuples {
// 	($t1:ident, $(($($t:ident),*),)? $tl:ident) => {

// #[allow(non_snake_case)]
// impl<$t1, $($($t,)*)? $tl> FromMutRequestHead for ($t1, $($($t,)*)? $tl)
// where
// 	$t1: FromMutRequestHead + Send,
// 	$($($t: FromMutRequestHead + Send,)*)?
// 	$tl: FromMutRequestHead + Send,
// {
// 	type Error = BoxedErrorResponse;
//
// 	async fn from_request_head(head: &mut RequestHead) -> Result<Self, Self::Error> {
// 		let $t1 = $t1::from_request_head(head).await.map_err(Into::into)?;
//
// 		$(
// 			$(
// 				let $t = $t::from_request_head(head).await.map_err(Into::into)?;
// 			)*
// 		)?
//
// 		let $tl = $tl::from_request_head(head).await.map_err(Into::into)?;
//
// 		Ok(($t1, $($($t,)*)? $tl))
// 	}
// }

// #[allow(non_snake_case)]
// impl<'r, B, $t1, $($($t,)*)? $tl> FromRequestRef<'r, B> for ($t1, $($($t,)*)? $tl)
// where
// 	B: Send + Sync,
// 	$t1: FromRequestRef<'r, B> + Send,
// 	$($($t: FromRequestRef<'r, B> + Send,)*)?
// 	$tl: FromRequestRef<'r, B> + Send,
// {
// 	type Error = BoxedErrorResponse;
//
// 	async fn from_request_ref(request: &'r Request<B>) -> Result<Self, Self::Error> {
// 		let $t1 = $t1::from_request_ref(request).await.map_err(Into::into)?;
//
// 		$(
// 			$(
// 				let $t = $t::from_request_ref(request).await.map_err(Into::into)?;
// 			)*
// 		)?
//
// 		let $tl = $tl::from_request_ref(request).await.map_err(Into::into)?;
//
// 		Ok(($t1, $($($t,)*)? $tl))
// 	}
// }

// #[allow(non_snake_case)]
// impl<$t1, $($($t,)*)? $tl, B> FromRequest<B> for ($t1, $($($t,)*)? $tl)
// where
// 	$t1: FromMutRequestHead + Send,
// 	$($($t: FromMutRequestHead + Send,)*)?
// 	$tl: FromRequest<B> + Send,
// 	B: Send,
// {
// 	type Error = BoxedErrorResponse;
//
// 	async fn from_request(request: Request<B>) -> Result<Self, Self::Error> {
// 		let (mut head, body) = request.into_parts();
//
// 		let $t1 = $t1::from_request_head(&mut head).await.map_err(Into::into)?;
//
// 		$($(
// 			let $t = $t::from_request_head(&mut head).await.map_err(Into::into)?;
// 		)*)?
//
// 		let request = Request::from_parts(head, body);
//
// 		let $tl = $tl::from_request(request).await.map_err(Into::into)?;
//
// 		Ok(($t1, $($($t,)*)? $tl))
// 	}
// }

// };
// }

// call_for_tuples!(impl_extractions_for_tuples!);

// --------------------------------------------------------------------------------
