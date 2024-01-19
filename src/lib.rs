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
mod data;
pub mod extension;
pub mod handler;
pub mod header;
mod middleware;
mod pattern;
pub mod request;
pub mod resource;
pub mod response;
mod routing;
