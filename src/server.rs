#![doc = include_str!("../docs/server.md")]

// ----------

use std::{io::Error as IoError, net::ToSocketAddrs, pin::pin, time::Duration};

#[cfg(feature = "tls")]
use std::sync::Arc;

use argan_core::{body::Body, request::Request, response::Response, BoxedError};
use hyper::{body::Incoming, service::Service};
use hyper_util::{
	rt::{TokioExecutor, TokioIo},
	server::{conn::auto::Builder, graceful::GracefulShutdown},
};
use tokio::net::TcpListener;

#[cfg(feature = "tls")]
use tokio_rustls::{rustls::ServerConfig as TlsServerConfig, TlsAcceptor};

use crate::common::CloneWithPeerAddr;

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

/// A high-level server type.
///
/// For examples, see the module [`doc`](crate::server).
pub struct Server {
	connection_builder: Builder<TokioExecutor>,
	some_shutdown_duration: Option<Duration>,
}

impl Server {
	/// Creates a new `Server` with the provided [`Builder`].
	///
	/// [`Builder`]: https://docs.rs/hyper-util/latest/hyper_util/server/conn/auto/struct.Builder.html
	pub fn new(connection_builder: Builder<TokioExecutor>) -> Self {
		Self {
			connection_builder,
			some_shutdown_duration: None,
		}
	}

	/// Sets the graceful shutdown duration. By default, the server shuts down immediately.
	pub fn with_graceful_shutdown_duration(mut self, duration: Duration) -> Self {
		self.some_shutdown_duration = Some(duration);

		self
	}

	/// Serves HTTP connections with the `service` on the first successfully
	/// bound listener address.
	///
	/// # Panics
	/// - if there is no valid address with an unused port to bind `TcpListener`
	/// - on Unix systems if getting a signal listener to listen to `SIGTERM`, has failed
	pub async fn serve<S, A>(&self, service: S, listener_addresses: A) -> Result<(), ServerError>
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
			connection_builder,
			some_shutdown_duration,
		} = self;

		#[cfg(not(feature = "tls"))]
		return serve(
			service,
			listener_addresses,
			connection_builder,
			*some_shutdown_duration,
		)
		.await;

		#[cfg(feature = "tls")]
		serve(
			service,
			listener_addresses,
			None,
			connection_builder,
			*some_shutdown_duration,
		)
		.await
	}

	/// Serves HTTPS connections with the `service` on the first successfully
	/// bound listener address.
	///
	/// TLS can be configured with [`TlsServerConfig`] (`TlsServerConfig` is an alias
	/// for [`rustls`]'s [`ServerConfig`]).
	///
	/// # Panics
	/// - if there is no valid address with an unused port to bind `TcpListener`
	/// - on Unix systems if getting a signal listener to listen to `SIGTERM`, has failed
	///
	/// [`rustls`]: https://docs.rs/rustls/latest/rustls/
	/// [`TlsServerConfig`]: https://docs.rs/rustls/latest/rustls/server/struct.ServerConfig.html
	/// [`ServerConfig`]: https://docs.rs/rustls/latest/rustls/server/struct.ServerConfig.html
	#[cfg(feature = "tls")]
	pub async fn serve_with_tls<S, A>(
		&self,
		service: S,
		listener_addresses: A,
		tls_server_config: TlsServerConfig,
	) -> Result<(), ServerError>
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
			connection_builder,
			some_shutdown_duration,
		} = self;

		serve(
			service,
			listener_addresses,
			Some(tls_server_config),
			connection_builder,
			*some_shutdown_duration,
		)
		.await
	}
}

// This function can be called with `some_tls_server_config` argument set to `None`
// to serve requests without TLS.
async fn serve<S, A>(
	service: S,
	listener_addresses: A,
	#[cfg(feature = "tls")] some_tls_server_config: Option<TlsServerConfig>,
	connection_builder: &Builder<TokioExecutor>,
	some_shutdown_duration: Option<Duration>,
) -> Result<(), ServerError>
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
	let mut addresses = listener_addresses.to_socket_addrs()?;
	let some_listener = loop {
		let Some(address) = addresses.next() else {
			panic!("no valid address with an unbound port given");
		};

		if let Ok(listener) = TcpListener::bind(address).await {
			break Some(listener);
		}
	};

	let Some(listener) = some_listener else {
		panic!("no valid address with an unbound port given");
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

	#[cfg(feature = "tls")]
	// When `some_tls_server_config` is `None` we should not use `TlsAcceptor`.
	let some_tls_acceptor =
		some_tls_server_config.map(|tls_server_config| TlsAcceptor::from(Arc::new(tls_server_config)));

	let graceful_shutdown_watcher = GracefulShutdown::new();

	loop {
		#[cfg(feature = "tls")]
		let some_tls_acceptor_clone = some_tls_acceptor.clone();

		tokio::select! {
			connection = listener.accept() => {
				match connection {
					Ok((stream, _peer_address)) => {
						#[cfg(feature = "tls")]
						if let Some(tls_acceptor_clone) = some_tls_acceptor_clone {
							// The `tls` feature flag is enabled, and the function is called
							// with TlsServerConfig.

							let stream = tls_acceptor_clone.accept(stream).await?;

							let connection = connection_builder.serve_connection_with_upgrades(
								TokioIo::new(stream),
								#[cfg(not(feature = "peer-addr"))]
								service.clone(),
								#[cfg(feature = "peer-addr")]
								service.clone_with_peer_addr(_peer_address),
							);

							let connection = graceful_shutdown_watcher.watch(connection.into_owned());

							tokio::spawn(connection);

							continue;
						}

						let connection = connection_builder.serve_connection_with_upgrades(
							TokioIo::new(stream),
							#[cfg(not(feature = "peer-addr"))]
							service.clone(),
							#[cfg(feature = "peer-addr")]
							service.clone_with_peer_addr(_peer_address),
						);

						let connection = graceful_shutdown_watcher.watch(connection.into_owned());

						tokio::spawn(connection);
					},
					Err(error) => {
						tokio::time::sleep(Duration::from_secs(1)).await;

						if accept_error_count < 3 {
							accept_error_count += 1;

							continue;
						}

						return Err(ServerError::from(error));
					}
				};
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

// --------------------------------------------------

/// An error type of server failures.
#[derive(Debug, crate::ImplError)]
#[error(transparent)]
pub struct ServerError(#[from] IoError);

// --------------------------------------------------
