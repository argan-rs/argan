A web framework.

## Resource

Argan applications are built by composing [`Resource`]s into a tree. Each resource in the tree
corresponds to one of the path segments of the URI. When a request comes, it passes through the
resources that match its URI path segments. If a resource that matches the last path segment
exists, that resource handles the request. Otherwise, a resource that matches the farthest
segment (not the last segment) responds with its custom or with the default *mistargeted
request handler* that returns a `"404 Not Found"` response.

```
use argan::{
    Resource,
    handler::_get,
};

//  "/resource_0_0" --- "/resource_1_0" --- "/resource_2_0"
//                 |                   |
//                 |                    --- "/resource_2_1" --- "/resource_3_0"
//                 |
//                  --- "/resource_1_1"

// Following resources form the above tree.

// A resource can be created with the pattern of its corresponding segment
// or an absolute path pattern.

let mut resource_0_0 = Resource::new("/resource_0_0");

let mut resource_1_1 = Resource::new("/resource_1_1");
resource_1_1.set_handler_for(_get(|| async { "resource_1_1" }));

let mut resource_2_0 = Resource::new("/resource_0_0/resource_1_0/resource_2_0");
resource_2_0.set_handler_for(_get(|| async { "resource_2_0" }));

resource_0_0.add_subresource([resource_1_1, resource_2_0]);

// We can also create a new subresource or get an existing one with a relative
// path pattern from a parent.

let mut resource_2_1 = resource_0_0.subresource_mut("/resource_1_0/resource_2_1");
resource_2_1.set_handler_for(_get(|| async { "resource_2_1" }));
resource_2_1
    .subresource_mut("/resource_3_0")
    .set_handler_for(_get(|| async { "resource_3_0" }));

let resource_service = resource_0_0.into_arc_service();
```

A `Resource` itself is mainly made of three components: *request receiver*, *request passer*,
and *request handler*. All three of them are [`Handler`]s and can be
wrapped with middleware.

The *request receiver* as its name suggests is responsible for receiving a request and handing
it over to the *request passer* if the resource is not the request's target resource, or to the
*request handler* if it is.

The *request passer* finds the next resource that matches the next path segment and passes the
request to that resource. If there is no matching resource, the *request passer* calls the
resource's custom or the default *mistargeted request handler* to generate a `"404 Not Found"`
response unless there is a *subtree handler* among the parent resources. In that case it returns
the request back to that *subtree handler* parent that can handle the request.

The *request handler* contains all the HTTP *method handler*s of the resource. It is responsible
for calling the corresponding *method handler* that matches the request's method.

## Host

A [`Host`] is another node in Argan that can be converted into a service. But unlike a [`Router`]
and [`Resource`] nodes, a [`Host`] doesn't have any [`Handler`] components. Instead it contains a
root resource and guards the resource tree against the request's `"Host"`.

```
use argan::{
    Host,
    Resource,
    handler::_get,
};

async fn hello_world() -> &'static str {
    "Hello, World!"
}

let mut root = Resource::new("/");
root.set_handler_for(_get(hello_world));

let host_service = Host::new("http://example.com", root).into_arc_service();
```

## Router

When there is a need for multiple resource trees with different host guards a [`Router`] node
can be used.

```
use argan::{
    Router,
    Host,
    Resource,
    handler::_get,
};

let mut router = Router::new();

let mut example_com_root = Resource::new("/");
example_com_root.set_handler_for(_get(|| async { "example.com" }));

let mut abc_example_com_root = Resource::new("/");
abc_example_com_root.set_handler_for(_get(|| async { "abc.example.com" }));

router.add_host([
    Host::new("http://example.com", example_com_root),
    Host::new("http://abc.example.com", abc_example_com_root),
]);

let mut bca_example_com_root = Resource::new("http://bca.example.com/");
bca_example_com_root.set_handler_for(_get(|| async { "bca.example.com" }));

// We can also add a hostless resource tree for requests with a "Host" that doesn't
// match any of our hosts.

let mut hostless_resource = Resource::new("/resource");
hostless_resource.set_handler_for(_get(|| async { "resource" }));

router.add_resource([bca_example_com_root, hostless_resource]);

let router_service = router.into_leaked_service();
```

## Pattern

There are three kinds of patterns in Argan: *static*, *regex*, and *wildcard*. Path segment
patterns can be of any kind while host pattern can only be *static* or *regex*. Hosts and
resources with *static* patterns have the highest priority. That is, when the *request passer*
is trying to find the next resource, first, it matches the next path segment against resources
with *static* patterns. Then comes resources with *regex* patterns and finally a resource with
a *wildcard* pattern.

*Static* pattern matches the request's path segment exactly.

```
use argan::Resource;

// Resources with static patterns.

let news = Resource::new("/news");
let items = Resource::new("/items");
```

*Regex* pattern can have *static* and *regex* subpatterns. *Regex* pattern matches if both
subpatterns match the request's path segment in the exact order. *Regex* subpatterns are written
in the curly braces with its name and regex parts separated by colon: `"{name:regex}"`. If the
regex subpattern is the last suppattern or the following subpattern is a *static* subpattern
that starts with a dot `'.'`, then regex part can be omitted to match anything.

```
use argan::{Host, Resource};

// Resources with regex patterns.

let sub_domain = Host::new("http://{sub}.example.com", Resource::new("/"));

// Here the `number_of_days` is a path parameter name, which can have a value of `5` or `10`
// and `-days-forecast` is a static subpattern.
let n_days_forecast = Resource::new("/{number_of_days:5|10}-days-forecast");

// Here `id:` is a static subpattern and the `prefix` and `number` are the path parameter names.
let id = Resource::new(r"/id:{prefix:A|B|C}{number:\d{5}}");

// Here we have `name` and `ext` path parameter names, Both match anything that's separated
// with a dot `.`.
let file = Resource::new("/{name}.{ext}");
```

*Wildcard* pattern matches anything in the request's path segment. *Wildcard* pattern has
only a name in the curly braces.

```
use argan::Resource;

// Resources with wildcard patterns.

let echo = Resource::new("/{echo}");
let black_box = Resource::new("/{black_box}");
```

A [`Resource`] may contain any number of child resources with a *static* and *regex* patterns.
But it can contain only one resource with a *wildcard* pattern. 

Patterns can be joined together to form a path pattern or a URI pattern. Note that a resource
with a prefix URI pattern should be used with a [`Router`]. Otherwise, its host pattern, if
exists also prefix segment patterns will be ignored when it is converted into service. Likewise
a resource with a prefix path pattern should be used as a subresource instead of being converted
into service.

```
use argan::Resource;

// A resource with only an its own segment pattern.
let resource = Resource::new("/products");

// A resource with a prefix path pattern. A path pattern is always considered
// as an absolute path.
let resource = Resource::new("/groups/{group_id}/users/{user_id}");

// A resource with a prfix URI pattern.
let resource = Resource::new("http://{lang:en|fr}.example.com/about");
```

A resource pattern may end with a trailing slash `/`, meaning the request targeting the resource
must also have a trailing slash in its path, if not, the resource redirects the request to a path
with a trailing slash. When the resource pattern doesn't have a trailing slash, but request's path
does, then the resource redirects the request to a path without a trailing slash. This is a default
behavior.

A resource pattern can also have configuration symbols, `!`, `?`, `*`, attached after a
single space. They can be used to configure the resource to drop or to handle the request
on the absence or presence of a trailing slash `/` in the request's path, and also to handle
the requests that target non-existent subresources, aka as a *subtree* handler. An `!` configures
the resource to drop the request when the absence or presence of a trailing slash in its path
doesn't match the resource's pattern. A `?` configures the resource to be lenient to a trailing
slash in the request's path, which means the resource in any case handles the request. `!` and `?`
configuration symbols are mutually exclusive. An `*` configures the resource as a *subtree*
handler. Configuration symbols can only be specified on the resource's own pattern. Prefix segment
patterns given when the resource is being created or retrieved cannot have configuration symbols.

| symbol(s) on a pattern | resource                                                               |
|------------------------|------------------------------------------------------------------------|
| `"/some_pattern"`      | redirects the requests with a trailing slash                           |
| `"/some_pattern/"`     | redirects the requests without a trailing slash                        |
| `"/some_pattern \*"`   | redirects the requests with a trailing slash; subtree handler          |
| `"/some_pattern/ \*"`  | redirects the requests without a trailing slash; subtree handler       |
| `"/some_pattern !"`    | sends a `"404"` when there is a trailing slash                         |
| `"/some_pattern/ !"`   | sends a `"404"` when there is no trailing slash                        |
| `"/some_pattern !\*"`  | sends a `"404"` when there is a tralling slash; subtree handler        | 
| `"/some_pattern/ !\*"` | sends a `"404"` when there is no tralling slash; subtree handler       |
| `"/some_pattern ?"`    | handles the requests with or without a trailing slash                  |
| `"/some_pattern/ ?"`   | handles the requests with or without a trailing slash                  |
| `"/some_pattern ?\*"`  | handles the requests with or without a trailing slash; subtree handler |
| `"/some_pattern/ ?\*"` | handles the requests with or without a trailing slash; subtree handler |

## Handler

A handler is a type that implements the [`Handler`] trait. In a high-level, Argan applications
use `async` functions. These functions take specific set of parameters in a specific order and
must return a value of a type that implements either the [`IntoResponse`] or [`ErrorResponse`]
traits. Following are the parameters in the required order: [`RequestHead`], [`FromRequest`]
implementor, [`Args`].

```
use argan::{
    request::RequestHead,
    data::Text,
    handler::Args,
};

use serde::Deserialize;

// `async` functions that can be used as request handlers.
// Note that `()` implements the `IntoResponse` trait and can be used
// as a "success" response with a "200 OK" status code and an empty body.

async fn no_params() {
    // ...
}

async fn with_request_head(head: RequestHead) {
    // ...
}

async fn with_from_request_implementor(Text(text): Text) {
    // ...
}

// In the following function `()` in the `Args` can be a type provided as a handler
// extension via the `IntoHandler::with_extension()` method.
async fn with_args(args: Args<'static, ()>) {
    // ...
}

async fn with_request_head_and_from_request(head: RequestHead, Text(text): Text) {
    // ...
}

async fn with_request_head_and_args(head: RequestHead, args: Args<'static, ()>) {
    // ...
}

async fn with_from_request_and_args(Text(text): Text, args: Args<'static, ()>) {
    // ...
}

async fn with_all_three(head: RequestHead, Text(text): Text, args: Args<'static, ()>) {
    // ...
}
```

In addition to the combinations of these three main parameters, handler functions can take
up to 12 parameters that implement [`ExtractorGuard`] trait. Those parameters must come before
the combinations of the three main parameters.

## Error handling

Errors in Argan can be handled in the handler functions or at some points in the node tree using
a middleware. If the handler decides to return an [`ErrorResponse`], that error passes through
the nodes in the node tree up to the first node that was converted into service. First node may
be a resource, a host, or a router. If the error reaches the service without being handled,
the service will convert it into [`Response`] before passing it to the [`hyper`]. 

An error handler middleware can be layered on the components of the [`Resource`] and [`Router`]
via the [`ErrorHandlerLayer`]. The constructor of the [`ErrorHandlerLayer`] takes a parameter
that implements the [`ErrorHandler`] trait.

## Middleware

Middleware can be applied to handlers and resource components using the [`Layer`] implementors.
In addition to its own trait for layers, Argan also supports Tower layers.

See [`Router::add_layer_to()`], [`Resource::add_layer_to()`], and
[`IntoHandler::wrapped_in()`](crate::handler::IntoHandler::wrapped_in()) for more information.

TODO.

[`Handler`]: crate::handler::Handler
[`Args`]: crate::handler::Args
[`ErrorHandler`]: crate::handler::ErrorHandler
[`ErrorHandlerLayer`]: crate::middleware::ErrorHandlerLayer
[`Layer`]: crate::middleware::Layer
[`Response`]: crate::response::Response
[`IntoResponse`]: crate::response::IntoResponse
[`ErrorResponse`]: crate::response::ErrorResponse
[`RequestHead`]: crate::request::RequestHead
[`FromRequest`]: crate::request::FromRequest
[`ExtractorGuard`]: crate::request::ExtractorGuard
[`hyper`]: https://docs.rs/hyper/latest/hyper/
