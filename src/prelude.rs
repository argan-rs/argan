//! A module of re-exported common types and traits.

#[doc(hidden)]
pub use crate::{
	common::{BoxedError, BoxedFuture},
	handler::{Args, BoxableHandler, ErrorHandler, Handler, HandlerSetter, IntoHandler},
	http::{
		CustomMethod, HeaderMap, HeaderName, HeaderValue, Method, StatusCode, Uri, Version,
		WildcardMethod,
	},
	middleware::{
		ErrorHandlerLayer, HandlerWrapper, IntoLayer, Layer, RedirectionLayer, RequestHandler,
		RequestPasser, RequestReceiver,
	},
	request::{
		ExtractorGuard, FromRequest, MistargetedRequest, PathParamsError, QueryParamsError, Request,
		RequestContext, RequestHead, RequestHeadParts,
	},
	response::{
		BoxedErrorResponse, ErrorResponse, Html, IntoResponse, IntoResponseHeadParts,
		IntoResponseResult, Redirect, Response, ResponseError, ResponseExtension,
		ResponseExtensionError, ResponseHeadParts, ResponseResult,
	},
	Host, Resource, Router, Server,
};
