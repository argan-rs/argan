#![forbid(unsafe_code)]
#![allow(dead_code)]
#![allow(unused)]

// --------------------------------------------------

pub(crate) use std::error::Error as StdError;
pub(crate) use thiserror::Error as ImplError;

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

#[macro_use]
pub mod common;

pub mod body;
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
