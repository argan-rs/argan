High-level server functionality.

To use [`Server`], add the [`hyper-util`] crate to dependencies with the desired features.

```no_run
use std::time::Duration;
use argan::{Resource, Server, http::Method, handler::HandlerSetter};
use hyper_util::{server::conn::auto::Builder, rt::TokioExecutor};

// --------------------------------------------------

# async fn run() -> Result<(), Box<dyn std::error::Error>> {
let mut root = Resource::new("/");
root.set_handler_for(Method::GET.to(|| async { "Hello, World!" }));

let connection_builder = Builder::new(TokioExecutor::new());

let _ = Server::new(connection_builder)
  .with_graceful_shutdown_duration(Duration::from_secs(5))
  .serve(root.into_arc_service(), "localhost:8000")
  .await?;

# Ok(())
# }
```

For TLS support, enable the `tls` feature flag. Also, add [`rustls`] and other helper
crates like [`rustls-pki-types`] and [`rustls-pemfile`] with the desired features.

```no_run
use std::{time::Duration, fs::File, io::{BufReader, Error as IoError}, path::Path};
use argan::{Resource, Server, http::Method, handler::HandlerSetter};
use hyper_util::{server::conn::auto::Builder, rt::TokioExecutor};
use rustls::ServerConfig as TlsServerConfig;
use rustls_pemfile::{certs, private_key};
use rustls_pki_types::{CertificateDer, PrivateKeyDer};

// --------------------------------------------------

fn load_certs<P: AsRef<Path>>(path: P) -> Result<Vec<CertificateDer<'static>>, IoError> {
  let file = File::open(path.as_ref())?;
  
  certs(&mut BufReader::new(file)).collect()
}

fn load_private_key<P: AsRef<Path>>(path: P) -> Result<PrivateKeyDer<'static>, IoError> {
  let file = File::open(path.as_ref())?;
  
  private_key(&mut BufReader::new(file))
    .and_then(|some_key| some_key.ok_or(IoError::new(std::io::ErrorKind::Other, NoPrivateKey)))
}

// -------------------------

#[derive(Debug)]
struct NoPrivateKey;

impl std::error::Error for NoPrivateKey {}

impl std::fmt::Display for NoPrivateKey {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.write_str("no private key")
  }
}

// -------------------------

# async fn run() -> Result<(), Box<dyn std::error::Error>> {
let mut root = Resource::new("/");
root.set_handler_for(Method::GET.to(|| async { "Hello, World!" }));

let connection_builder = Builder::new(TokioExecutor::new());

let cert_chain = load_certs("cert.pem")?;
let private_key = load_private_key("privkey.pem")?;

let tls_server_config = TlsServerConfig::builder()
  .with_no_client_auth()
  .with_single_cert(cert_chain, private_key)?;

let _ = Server::new(connection_builder)
  .with_graceful_shutdown_duration(Duration::from_secs(5))
  .serve_with_tls(
    root.into_arc_service(),
    "localhost:8000",
    tls_server_config,
  )
  .await?;

# Ok(())
# }
```

[`hyper-util`]: https://crates.io/crates/hyper-util
[`rustls`]: https://crates.io/crates/rustls
[`rustls-pki-types`]: https://crates.io/crates/rustls-pki-types
[`rustls-pemfile`]: https://crates.io/crates/rustls-pemfile
