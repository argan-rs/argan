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
pub mod header;
mod middleware;
mod pattern;
pub mod request;
pub mod resource;
pub mod response;
mod routing;
