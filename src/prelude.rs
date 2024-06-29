//! Re-exported types and traits for convinience.

#[doc(inline)]
pub use crate::{
	common::{BoxedError, BoxedFuture},
	handler::{Args, BoxableHandler, ErrorHandler, Handler, HandlerSetter, IntoHandler},
	http::{
		CustomMethod, HeaderMap, HeaderName, HeaderValue, Method, StatusCode, Uri, Version,
		WildcardMethod,
	},
	middleware::{
		ErrorHandlerLayer, HandlerWrapper, IntoLayer, Layer, RequestHandler, RequestPasser,
		RequestReceiver,
	},
	request::{
		ExtractorGuard, FromRequest, MistargetedRequest, PathParamsError, Request, RequestContext,
		RequestHead, RequestHeadParts,
	},
	response::{
		BoxedErrorResponse, ErrorResponse, Html, IntoResponse, IntoResponseHeadParts,
		IntoResponseResult, Redirect, Response, ResponseError, ResponseExtension,
		ResponseExtensionError, ResponseHeadParts, ResponseResult,
	},
	Host, Resource, Router, Server,
};

#[cfg(feature = "query-params")]
pub use crate::request::QueryParamsError;
