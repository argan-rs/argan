use std::future::Future;

pub use tower::{
	util::{service_fn, ServiceFn},
	Service,
};

use super::utils::*;

// --------------------------------------------------

pub trait IntoService<S, Req>
where
	S: Service<Req>,
{
	fn into_service(self) -> S;
}

// -------------------------

impl<Func, F> IntoService<ServiceFn<Func>, Request> for Func
where
	Func: FnMut(Request) -> F,
	F: Future<Output = Result<Response, BoxedError>>,
{
	fn into_service(self) -> ServiceFn<Func> {
		service_fn(self)
	}
}

impl<T, Req> IntoService<T, Req> for T
where
	T: Service<Req>,
{
	fn into_service(self) -> T {
		self
	}
}

// --------------------------------------------------

#[cfg(test)]
mod test {
	use super::*;

	fn is_service<T, S, Req>(_: T)
	where
		T: IntoService<S, Req>,
		S: Service<Req>,
	{
	}

	async fn handler(_: Request) -> Result<Response, BoxedError> {
		unimplemented!()
	}

	async fn handler_mut(mut _unused: Request) -> Result<Response, BoxedError> {
		unimplemented!()
	}

	fn handler_returning_future(_: Request) -> BoxPinnedFuture {
		unimplemented!()
	}

	#[test]
	fn test() {
		is_service(handler);
		is_service(handler_mut);
		is_service(handler_returning_future)
	}
}
