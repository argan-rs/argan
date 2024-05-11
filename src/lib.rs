// ----------
#![doc = include_str!("../docs/argan.md")]
// ----------
#![allow(dead_code)]
// ----------

pub(crate) use std::error::Error as StdError;
pub(crate) use thiserror::Error as ImplError;

pub use argan_core::body;

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

#[macro_use]
pub mod common;

pub mod data;
pub mod handler;

pub mod host;
#[doc(inline)]
pub use host::Host;

pub mod middleware;
mod pattern;
pub mod request;

pub mod resource;
#[doc(inline)]
pub use resource::Resource;
#[cfg(features = "file-stream")]
pub use resource::StaticFiles;

pub mod response;

pub mod router;
#[doc(inline)]
pub use router::Router;
