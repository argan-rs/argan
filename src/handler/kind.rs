use std::str::FromStr;

use argan_core::body::Body;
use http::Method;

use crate::{
	middleware::{IntoResponseResultAdapter, ResponseResultFutureBoxer},
	response::{BoxedErrorResponse, IntoResponse},
};

use super::{BoxedHandler, FinalHandler, Handler, IntoHandler};

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

mod private {
	use argan_core::IntoArray;

	use super::*;

	#[allow(private_interfaces)]
	pub enum HandlerKind {
		Method(Method, BoxedHandler),
		WildcardMethod(Option<BoxedHandler>),
		MistargetedRequest(BoxedHandler),
	}

	impl IntoArray<HandlerKind, 1> for HandlerKind {
		fn into_array(self) -> [HandlerKind; 1] {
			[self]
		}
	}
}

pub(crate) use private::HandlerKind;

// --------------------------------------------------

macro_rules! handler_kind_by_method {
	($func:ident, $http_method:path) => {
		pub fn $func<H, Mark>(handler: H) -> HandlerKind
		where
			H: IntoHandler<Mark, Body>,
			H::Handler: Handler + Clone + Send + Sync + 'static,
			<H::Handler as Handler>::Response: IntoResponse,
			<H::Handler as Handler>::Error: Into<BoxedErrorResponse>,
			<H::Handler as Handler>::Future: Send,
		{
			let final_handler =
				ResponseResultFutureBoxer::wrap(IntoResponseResultAdapter::wrap(handler.into_handler()));

			HandlerKind::Method($http_method, final_handler.into_boxed_handler())
		}
	};
}

handler_kind_by_method!(_get, Method::GET);
handler_kind_by_method!(_head, Method::HEAD);
handler_kind_by_method!(_post, Method::POST);
handler_kind_by_method!(_put, Method::PUT);
handler_kind_by_method!(_patch, Method::PATCH);
handler_kind_by_method!(_delete, Method::DELETE);
handler_kind_by_method!(_options, Method::OPTIONS);
handler_kind_by_method!(_connect, Method::CONNECT);
handler_kind_by_method!(_trace, Method::TRACE);

pub fn _method<M, H, Mark>(method: M, handler: H) -> HandlerKind
where
	M: AsRef<str>,
	H: IntoHandler<Mark, Body>,
	H::Handler: Handler + Clone + Send + Sync + 'static,
	<H::Handler as Handler>::Response: IntoResponse,
	<H::Handler as Handler>::Error: Into<BoxedErrorResponse>,
	<H::Handler as Handler>::Future: Send,
{
	let method = Method::from_str(method.as_ref())
		.expect("HTTP method should be a valid token [RFC 9110, 5.6.2 Tokens]");

	let final_handler =
		ResponseResultFutureBoxer::wrap(IntoResponseResultAdapter::wrap(handler.into_handler()));

	HandlerKind::Method(method, final_handler.into_boxed_handler())
}

pub fn _wildcard_method<H, Mark>(some_handler: Option<H>) -> HandlerKind
where
	H: IntoHandler<Mark, Body>,
	H::Handler: Handler + Clone + Send + Sync + 'static,
	<H::Handler as Handler>::Response: IntoResponse,
	<H::Handler as Handler>::Error: Into<BoxedErrorResponse>,
	<H::Handler as Handler>::Future: Send,
{
	let some_final_handler = some_handler.map(|handler| {
		let handler = handler.into_handler();

		ResponseResultFutureBoxer::wrap(IntoResponseResultAdapter::wrap(handler)).into_boxed_handler()
	});

	HandlerKind::WildcardMethod(some_final_handler)
}

pub fn _mistargeted_request<H, Mark>(handler: H) -> HandlerKind
where
	H: IntoHandler<Mark, Body>,
	H::Handler: Handler + Clone + Send + Sync + 'static,
	<H::Handler as Handler>::Response: IntoResponse,
	<H::Handler as Handler>::Error: Into<BoxedErrorResponse>,
	<H::Handler as Handler>::Future: Send,
{
	let final_handler =
		ResponseResultFutureBoxer::wrap(IntoResponseResultAdapter::wrap(handler.into_handler()));

	HandlerKind::MistargetedRequest(final_handler.into_boxed_handler())
}

// --------------------------------------------------------------------------------

// #[cfg(test)]
// mod test {
// 	use std::{convert::Infallible, future::ready, marker::PhantomData};
//
// 	use argan_core::{
// 		request::{FromRequest, FromRequestHead, Request},
// 		response::Response,
// 		BoxedFuture,
// 	};
// 	use http::{Extensions, Uri};
//
// 	use crate::{
// 		common::marker::Private,
// 		data::{extensions::NodeExtension, form::Multipart},
// 		handler::Args,
// 		request::{PathParams, RemainingPath},
// 		routing::{RouteTraversal, RoutingState},
// 	};
//
// 	use super::*;
//
// 	// --------------------------------------------------------------------------------
// 	// --------------------------------------------------------------------------------
//
// 	#[derive(Clone)]
// 	struct H<T1>(PhantomData<T1>);
// 	impl<'n, T1: FromRequest<Body, Args<'n, ()>>> Handler for H<T1> {
// 		type Response = Response;
// 		type Error = Infallible;
// 		type Future = BoxedFuture<Result<Self::Response, Self::Error>>;
//
// 		fn handle(&self, request: Request<Body>, args: Args<'_, ()>) -> Self::Future {
// 			Box::pin(ready(Ok(Response::default())))
// 		}
// 	}
//
// 	struct Boo<T1>(PhantomData<T1>);
//
// 	impl<T1> IntoHandler<Private> for Boo<T1>
// 	where
// 		H<T1>: Handler,
// 	{
// 		type Handler = H<T1>;
//
// 		fn into_handler(self) -> Self::Handler {
// 			H::<T1>(PhantomData)
// 		}
// 	}
//
// 	fn is_from_request<'a, F2: FromRequestHead<Args<'a, ()>>, F1: FromRequest<Body, Args<'a, ()>>>() {
// 	}
//
// 	fn is_into_handler<H, Mark>(h: H)
// 	where
// 		H: IntoHandler<Mark, Body>,
// 		H::Handler: Handler + Clone + Send + Sync + 'static,
// 		<H::Handler as Handler>::Response: IntoResponse,
// 		<H::Handler as Handler>::Error: Into<BoxedErrorResponse>,
// 		<H::Handler as Handler>::Future: Send,
// 	{
// 		let _ = h.into_handler();
// 	}
//
// 	#[test]
// 	fn ttt_test() {
// 		is_from_request::<RemainingPath, PathParams<String>>();
// 		is_from_request::<Method, Uri>();
// 		is_from_request::<Method, RemainingPath>();
// 		is_from_request::<PathParams<String>, Request>();
//
// 		// let boo = Boo::<RemainingPath>(PhantomData);
// 		// is_into_handler(boo);
//
// 		let boo = Boo::<Method>(PhantomData);
// 		is_into_handler(boo);
//
// 		// let boo = Boo::<Request>(PhantomData);
// 		// is_into_handler(boo);
//
// 		is_into_handler(|_: PathParams<String>, _: RemainingPath, _: Request| async {});
// 		is_into_handler(|_: NodeExtension<String>, _: Request| async {});
// 		is_into_handler(|_: NodeExtension<String>, _: Multipart| async {});
// 		is_into_handler(|_: Multipart| async {});
// 		is_into_handler::<_, Request>(|_: Request| async {});
// 	}
// }
