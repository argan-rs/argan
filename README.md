# Argan

[![Crates.io](https://img.shields.io/crates/v/argan)](https://crates.io/crates/argan)
[![Test status](https://github.com/argan-rs/argan/actions/workflows/CI.yml/badge.svg?branch=main)](https://github.com/argan-rs/argan/actions/workflows/CI.yml)
[![Documentation](https://docs.rs/argan/badge.svg)](https://docs.rs/argan)

A web framework for Rust.

## Features

  * Static, regex, and wildcard URI component patterns.
  * Resource and handler extensions.
  * Request data extractors and support for custom guards.
  * Static file streaming with support for `multipart/byteranges`, pre-encoded files,
    and dynamic encoding.
  * Server-sent events.
  * WebSockets.
  * Flexible middleware system compatible with Tower.
  * Flexible error handling.



## Usage example

Cargo.toml

```Rust
[dependencies]
argan = { version = "0.0.1", features = ["json", "form"] }
hyper-util = { version = "0.1", features = ["server-auto", "tokio", "service"] }
serde = { version = "1", features = ["derive"] }
tokio = { version = "1", features = ["net", "rt-multi-thread", "macros"] }
```

main.rs

```Rust
use std::net::SocketAddr;

use argan::{
	common::BoxedError,
	data::{form::Form, json::Json},
	handler::{_get, _post, _wildcard_method},
	request::RequestHead,
	response::Redirect,
	Router,
};

use hyper_util::{
	rt::{TokioExecutor, TokioIo},
	server::conn::auto as server,
};
use serde::{Deserialize, Serialize};
use tokio::net::TcpListener;

#[tokio::main]
async fn main() -> Result<(), BoxedError> {
	let mut router = Router::new();

	let root = router.resource_mut("http://www.example.com/");
	root.set_handler_for(_get.to(|| async { "Hello, World!" }));

	root.subresource_mut("/login").set_handler_for(_post.to(login));

	router
		.resource_mut("http://example.com/")
		.set_handler_for(_wildcard_method.to(Some(|head: RequestHead| async move {
			let path = head.uri_ref().path();

			Redirect::permanently_to(format!("http://www.example.com{}", path))
		})));

	let arc_service = router.into_arc_service();

	let addr = SocketAddr::from(([127, 0, 0, 1], 8000));
	let listener = TcpListener::bind(addr).await?;

	loop {
		let (stream, _) = listener.accept().await?;

		let io = TokioIo::new(stream);
		let arc_service = arc_service.clone();

		tokio::task::spawn(async move {
			let _ = server::Builder::new(TokioExecutor::new())
				.serve_connection(io, arc_service)
				.await;
		});
	}
}

async fn login(Form(_credential): Form<Credentials>) -> Json<Token> {
	// ...

	let token = Token {
		jwt: "JWT".to_owned(),
	};

	Json(token)
}

#[derive(Deserialize)]
struct Credentials {
	login: String,
	password: String,
}

#[derive(Serialize)]
struct Token {
	jwt: String,
}
```

## Contributions

Pull requests are accepted on the `dev` branch.

Any contribution intentionally submitted for inclusion in Argan by you shall be dual licensed
under the MIT License and Apache License, Version 2.0, like Argan, without any additional terms
or conditions.

## License

Argan is dual-licensed under either the [MIT License](LICENSE-MIT) or
[Apache License, Version 2.0](LICENSE-APACHE), at your option.
