use std::{future::Future, pin::Pin};

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

pub type BoxedError = Box<dyn std::error::Error + Send + Sync>;
pub type BoxedFuture<T> = Pin<Box<dyn Future<Output = T>>>;
