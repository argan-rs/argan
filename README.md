# Argan

[![Crates.io](https://img.shields.io/crates/v/argan)](https://crates.io/crates/argan)
[![Test status](https://github.com/argan-rs/argan/actions/workflows/CI.yml/badge.svg?branch=main)](https://github.com/argan-rs/argan/actions/workflows/CI.yml)
[![Documentation](https://docs.rs/argan/badge.svg)](https://docs.rs/argan)

A web framework for Rust.

## Features

  * Static, regex, and wildcard URI component patterns.
  * Resource and handler extensions.
  * Request data extractors and support for custom guards.
  * Static file streaming with support for range requests, `multipart/byteranges`,
    pre-encoded files, and dynamic encoding.
  * Server-sent events.
  * WebSockets.
  * Flexible middleware system compatible with Tower.
  * Flexible error handling.



## Usage example

Cargo.toml

```Rust
[dependencies]
argan = "0.0.2"
hyper-util = { version = "0.1", features = ["server-auto", "tokio"] }
serde = { version = "1", features = ["derive"] }
tokio = { version = "1", features = ["rt-multi-thread", "macros"] }
```

main.rs

```Rust
use std::time::Duration;

use argan::{
	data::{form::Form, json::Json},
	middleware::RedirectionLayer,
	prelude::*,
};

use hyper_util::{rt::TokioExecutor, server::conn::auto::Builder};
use serde::{Deserialize, Serialize};

// --------------------------------------------------------------------------------

#[derive(Deserialize)]
struct Credentials {
	// ...
}

#[derive(Serialize)]
struct Token {
	jwt: String,
}

async fn login(Form(credential): Form<Credentials>) -> Json<Token> {
	// ...

	let token = Token {
		jwt: "JWT".to_owned(),
	};

	Json(token)
}

async fn echo(request_head: RequestHead) -> String {
	request_head.subtree_path_segments().to_owned()
}

// --------------------------------------------------

#[tokio::main]
async fn main() -> Result<(), BoxedError> {
	let mut router = Router::new();

	let mut root = Resource::new("http://www.example.com/");
	root.set_handler_for(Method::GET.to(|| async { "Hello, World!" }));

	// Subresources can be created from a parent node (`Resource`, `Router`).
	root
		.subresource_mut("/login")
		.set_handler_for(Method::POST.to(login));

	router.add_resource(root);

	router
		.resource_mut("http://example.com/")
		.wrap(
			RequestReceiver.component_in(RedirectionLayer::for_permanent_redirection_to_prefix(
				"http://www.example.com/",
			)),
		);

	// A hostless resource responds to requests targeting any other host that
	// the router doesn't include.
	//
	// With the question mark '?' following its pattern, the resource accepts
	// a request with or without a trailing slash '/'. With asterisk '*', it
	// accepts requests that target non-existent subresources.
	router
		.resource_mut("/echo ?*")
		.set_handler_for(Method::GET.to(echo));

	let arc_service = router.into_arc_service();

	let connection_builder = Builder::new(TokioExecutor::new());

	Server::new(connection_builder)
		.with_graceful_shutdown_duration(Duration::from_secs(3))
		.serve(arc_service, "127.0.0.1:8000")
		.await
		.map_err(Into::into)
}
```

## Note

Currently, Argan is not tested on Windows.

## Contributions

Any contribution intentionally submitted for inclusion in Argan by you shall be dual licensed
under the MIT License and Apache License, Version 2.0, like Argan, without any additional terms
or conditions.

## License

Argan is dual-licensed under either the [MIT License](LICENSE-MIT) or
[Apache License, Version 2.0](LICENSE-APACHE), at your option.
