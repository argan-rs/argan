use std::{net::ToSocketAddrs, pin::pin, time::Duration};

use argan_core::{body::Body, request::Request, response::Response, BoxedError};
use hyper::{body::Incoming, service::Service};
use hyper_util::{
	rt::{TokioExecutor, TokioIo},
	server::{conn::auto::Builder as HyperServer, graceful::GracefulShutdown},
};
use tokio::net::TcpListener;

use crate::common::CloneWithPeerAddr;

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

pub struct Server {
	hyper_server: HyperServer<TokioExecutor>,
	graceful_shutdown_watcher: GracefulShutdown,
	some_shutdown_duration: Option<Duration>,
}

impl Server {
	pub fn new() -> Self {
		Self {
			hyper_server: HyperServer::new(TokioExecutor::new()),
			graceful_shutdown_watcher: GracefulShutdown::new(),
			some_shutdown_duration: None,
		}
	}

	pub fn with_graceful_shutdown_duration(mut self, duration: Duration) -> Self {
		self.some_shutdown_duration = Some(duration);

		self
	}

	pub async fn serve<S, A>(self, service: S, bind_address: A) -> Result<(), BoxedError>
	where
		S: Service<Request<Incoming>, Response = Response<Body>>
			+ CloneWithPeerAddr
			+ Clone
			+ Send
			+ 'static,
		S::Future: Send + 'static,
		S::Error: Into<BoxedError>,
		A: ToSocketAddrs,
	{
		let Server {
			hyper_server,
			graceful_shutdown_watcher,
			some_shutdown_duration,
		} = self;

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

		let mut accept_error_count = 0;
		let mut pinned_ctrl_c = pin!(tokio::signal::ctrl_c());

		#[cfg(unix)]
		let mut signal = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
			.expect("couldn't get the unix signal listener");

		#[cfg(unix)]
		let mut pinned_terminate = pin!(signal.recv());

		#[cfg(not(unix))]
		let mut pinned_terminate = pin!(std::future::pending::<()>());

		loop {
			tokio::select! {
				connection = listener.accept() => {
					let (stream, _peer_address) = match connection {
						Ok(connection) => connection,
						Err(error) => {
							tokio::time::sleep(Duration::from_secs(1)).await;

							if accept_error_count < 3 {
								accept_error_count += 1;

								continue;
							}

							return Err(error.into());
						}
					};

					let connection = hyper_server.serve_connection_with_upgrades(
						TokioIo::new(stream),
						#[cfg(not(feature = "peer-addr"))]
						service.clone(),
						#[cfg(feature = "peer-addr")]
						service.clone_with_peer_addr(_peer_address),
					);

					let connection = graceful_shutdown_watcher.watch(connection.into_owned());

					tokio::spawn(connection); // TODO: Do something with the error.
				},
				_ = pinned_ctrl_c.as_mut() => break,
				_ = pinned_terminate.as_mut() => break,
			}
		}

		if let Some(duration) = some_shutdown_duration {
			tokio::select! {
				_ = graceful_shutdown_watcher.shutdown() => {},
				_ = tokio::time::sleep(duration) => {},
			}
		}

		Ok(())
	}
}

impl Default for Server {
	fn default() -> Self {
		Self::new()
	}
}

#[derive(Debug, crate::ImplError)]
#[error("no valid address to bind")]
struct ServeError;

// --------------------------------------------------
