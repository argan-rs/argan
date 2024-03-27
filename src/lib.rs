#![allow(dead_code)]
#![allow(unused)]

// --------------------------------------------------

pub use argan_core::{body, StdError};
pub(crate) use thiserror::Error as ImplError;

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

#[macro_use]
pub mod common;

pub mod data;
pub mod handler;
pub mod host;
pub mod middleware;
mod pattern;
pub mod request;
pub mod resource;
pub mod response;
pub mod router;
mod routing;
