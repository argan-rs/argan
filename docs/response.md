HTTP response types.

In Argan, handlers return either a success response or an error response. That is, handler
functions must return a type that implements the [`IntoResponseResult`] trait. All types that
implement [`IntoResponse`] also blanketly implement [`IntoResponseResult`]. Errors must implement
both [`IntoResponse`] and [`Error`](std::error::Error) traits, and they should be returned as
the value of [`Err`].

```
use argan::{
    Resource,
    http::{Method, StatusCode},
    handler::HandlerSetter,
    request::RequestHead,
    response::{IntoResponse, IntoResponseResult, ResponseResult, ErrorResponse, ResponseError},
};
 
// `&'static str` implements the `IntoResponse` trait.
async fn hello() -> &'static str {
    "Hello, World!"
}

// If the error response should be logged or handled in some way, it
// must be returned as the value of `Err`. 
async fn bye() -> Result<(), ResponseError> {
    Err(ResponseError::from(StatusCode::BAD_REQUEST))
}

// If there is a need to return different success and error response types,
// they can be converted into `ResponseResult` before being returned.
async fn conditional_hello(request_head: RequestHead) -> ResponseResult {
    let mut some_condition = false;
    
    // ...
    
    if some_condition {
        return ResponseError::from(StatusCode::UNAUTHORIZED).into_error_result(); // Err
    }

    // If deserialization fails, `query_params_as()` returns `QueryParamsError`.
    let (value1, value2) = request_head.query_params_as::<(&str, u64)>()?;

    // ...

    if some_condition {
        return StatusCode::NO_CONTENT.into_response_result(); // Ok
    }

    "Hello!".into_response_result() // Ok
}

// ...

let mut root = Resource::new("/");
root.set_handler_for([Method::GET.to(hello), Method::POST.to(bye)]);

root
    .subresource_mut("/hello")
    .set_handler_for(Method::GET.to(conditional_hello));
```

[`IntoResponseResult`] is also implemented for tuple types with up to 16 elements. All
elements of a tuple except the first and last one must be of types that implement the
[`IntoResponseHeadParts`] trait. The first element of a tuple can be [`StatusCode`].
The last element's type must implement [`IntoResponseResult`]. 

```
use argan::{
    Resource,
    http::Method,
    handler::HandlerSetter,
    response::{IntoResponse, IntoResponseHeadParts, IntoResponseResult},
};

async fn hello() -> impl IntoResponseResult {
    ([("header_1", "value_1"), ("header_2", "value_2")], "Hello, World!")
}

// ...

let mut root = Resource::new("/");
root.set_handler_for(Method::GET.to(hello));
```

In the above example, `[("header_1", "value_1"), ("header_2", "value_2")]`, an array of header
name and header value tuples implements the [`IntoResponseResult`] trait.

Following is an example with [`StatusCode`].

```
use argan::{
    Resource,
    http::{StatusCode, Method, HeaderMap},
    handler::HandlerSetter,
    request::RequestHead,
    response::{IntoResponse, IntoResponseHeadParts, IntoResponseResult},
};

async fn hello(request_head: RequestHead) -> impl IntoResponseResult {
    let mut cookies = request_head.cookies();
    let mut headers = HeaderMap::new();
    
    // ...

    (StatusCode::OK, headers, cookies, "Hello, World!")
}

// ...

let mut root = Resource::new("/");
root.set_handler_for(Method::GET.to(hello));
```


