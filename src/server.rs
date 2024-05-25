use std::{error::Error, fmt::Display, net::ToSocketAddrs, time::Duration};

use argan_core::{body::Body, request::Request, response::Response, BoxedError};
use hyper::{body::Incoming, service::Service};
use hyper_util::{
	rt::{TokioExecutor, TokioIo},
	server::conn::auto::Builder as HyperServer,
};
use tokio::net::TcpListener;

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

pub struct Server {
	hyper_server: HyperServer<TokioExecutor>,
}

impl Server {
	pub fn new() -> Self {
		Self {
			hyper_server: HyperServer::new(TokioExecutor::new()),
		}
	}

	pub async fn serve<S, A>(&self, service: S, bind_address: A) -> Result<(), BoxedError>
	where
		S: Service<Request<Incoming>, Response = Response<Body>> + Clone + Send + 'static,
		S::Future: Send + 'static,
		S::Error: Into<BoxedError>,
		A: ToSocketAddrs,
	{
		let mut addresses = bind_address.to_socket_addrs()?;
		let some_listener = loop {
			let Some(address) = addresses.next() else {
				return Err(ServeError.into());
			};

			if let Ok(listener) = TcpListener::bind(address).await {
				break Some(listener);
			}
		};

		let Some(listener) = some_listener else {
			return Err(ServeError.into());
		};

		loop {
			let (stream, _) = listener.accept().await?;

			let server_clone = self.hyper_server.clone();
			let io = TokioIo::new(stream);
			let service = service.clone();

			tokio::spawn(async move {
				server_clone
					.serve_connection_with_upgrades(io, service)
					.await
			});
		}
	}
}

#[derive(Debug, crate::ImplError)]
#[error("no valid address to bind")]
struct ServeError;

// --------------------------------------------------
