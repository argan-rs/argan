#![allow(dead_code)]

// ----------

use std::{future::Future, pin::Pin};

// ----------

pub use std::error::Error as StdError;

use extensions::NodeExtensions;
use http::Extensions;
pub(crate) use thiserror::Error as ImplError;

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

#[macro_use]
pub(crate) mod macros;

pub mod body;
pub mod extensions;
pub mod request;
pub mod response;

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

pub type BoxedError = Box<dyn StdError + Send + Sync>;
pub type BoxedFuture<T> = Pin<Box<dyn Future<Output = T> + Send>>;

// --------------------------------------------------------------------------------

// --------------------------------------------------
// Args

#[non_exhaustive]
pub struct Args<'n, PrivateExt, HandlerExt> {
	pub(crate) private_extension: PrivateExt,
	pub node_extensions: NodeExtensions<'n>,
	pub handler_extension: &'n HandlerExt,
}

impl<'n, PrivateExt, HandlerExt> Args<'n, PrivateExt, HandlerExt> {
	pub fn new(private_extension: PrivateExt, handler_extension: &'n HandlerExt) -> Self {
		Self {
			private_extension,
			node_extensions: NodeExtensions::Owned(Extensions::new()),
			handler_extension,
		}
	}

	#[inline(always)]
	pub fn private_extension_mut(&mut self) -> &mut PrivateExt {
		&mut self.private_extension
	}
}

// --------------------------------------------------
// Arguments

// pub trait Arguments<'n, HandlerExt = ()>: Sized {
// 	fn private_extension(&mut self) -> &mut impl PrivateType;
// 	fn node_extension<Ext: Send + Sync + 'static>(&self) -> Option<&'n Ext>;
// 	fn handler_extension(&self) -> &'n HandlerExt;
// }

// --------------------------------------------------
// PrivateType marker

// pub trait PrivateType {}

// --------------------------------------------------
// IntoArray trait

pub trait IntoArray<T, const N: usize> {
	fn into_array(self) -> [T; N];
}

impl<T, const N: usize> IntoArray<T, N> for [T; N]
where
	T: IntoArray<T, 1>,
{
	fn into_array(self) -> [T; N] {
		self
	}
}

// --------------------------------------------------
// Marker

pub(crate) mod marker {
	pub struct Private;
}

// --------------------------------------------------
// Used when expecting a valid value in Options or Results.
pub(crate) const SCOPE_VALIDITY: &'static str = "scope validity";

// --------------------------------------------------------------------------------

// struct He;
// struct Bo;
//
// trait FromHead {}
// trait FromBody {}
// trait FromReq<Mark = Bo> {}
//
// impl<T: FromHead> FromReq<He> for T {}
// impl<T: FromBody> FromReq<Bo> for T {}
//
// // -----
//
// trait Hand {}
//
// struct Ha;
// impl Hand for Ha {}
//
// // -----
//
// trait IntoHand<Mark>: Sized {
// 	type Hand: Hand;
//
// 	fn into_hand(self) -> Self::Hand;
// }
//
// impl<Func, Mark, T> IntoHand<(Mark, T)> for Func
// where
// 	Func: Fn(T),
// 	T: FromReq<Mark>,
// {
// 	type Hand = Ha;
//
// 	fn into_hand(self) -> Self::Hand {
// 	  Ha
// 	}
// }
//
// impl<Func, T1, T2> IntoHand<(He, Bo, (T1, T2))> for Func
// where
// 	Func: Fn(T1, T2),
// 	T1: FromHead,
// 	T2: FromBody,
// {
// 	type Hand = Ha;
//
// 	fn into_hand(self) -> Self::Hand {
// 	  Ha
// 	}
// }
//
// impl<Func, T1, T2, T3> IntoHand<(He, Bo, (T1, T2, T3))> for Func
// where
// 	Func: Fn(T1, T2, T3),
// 	T1: FromHead,
// 	T2: FromHead,
// 	T3: FromBody,
// {
// 	type Hand = Ha;
//
// 	fn into_hand(self) -> Self::Hand {
// 	  Ha
// 	}
// }
//
// impl<Func, T1, T2> IntoHand<(He, He, (T1, T2))> for Func
// where
// 	Func: Fn(T1, T2),
// 	T1: FromHead,
// 	T2: FromHead,
// {
// 	type Hand = Ha;
//
// 	fn into_hand(self) -> Self::Hand {
// 	  Ha
// 	}
// }
//
// impl<Func, T1, T2, T3> IntoHand<(He, He, (T1, T2, T3))> for Func
// where
// 	Func: Fn(T1, T2, T3),
// 	T1: FromHead,
// 	T2: FromHead,
// 	T3: FromHead,
// {
// 	type Hand = Ha;
//
// 	fn into_hand(self) -> Self::Hand {
// 	  Ha
// 	}
// }
//
// // -------------------------
//
// fn from_req<Mark, T: FromReq<Mark>>(_: T) {}
// fn from_head_body<T1: FromHead, T2: FromBody>(_: T1, _: T2) {}
// fn from_head_head<T1: FromHead, T2: FromHead>(_: T1, _: T2) {}
//
// struct A;
// impl FromHead for A {}
//
// struct B;
// impl FromBody for B {}
//
// fn is_handler<Mark, I: IntoHand<Mark>>(_: I) {}
//
// fn test() {
// 	from_req(A);
// 	from_req(B);
//
// 	from_head_body(A, B);
// 	from_head_head(A, A);
//
// 	is_handler(|A| {});
// 	is_handler(|A, A| {});
// 	is_handler(|A, A, A| {});
// 	is_handler(|B| {});
// 	is_handler(|A, B| {});
// 	is_handler(|A, A, B| {});
// }
