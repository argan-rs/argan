#![allow(dead_code)]
#![allow(unused)]

#[macro_use]
pub(crate) mod macros;

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
mod utils;
