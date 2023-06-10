use std::{future::Future, pin::Pin};

use hyper::StatusCode;

use crate::{
	request::Request,
	response::Response,
	routing::{RoutingState, UnusedRequest},
	service::BoxedService,
};

use super::utils::*;

// --------------------------------------------------

pub struct Resource {
	name: &'static str,

	static_resources: Option<Vec<Resource>>,
	pattern_resources: Option<Vec<Resource>>,
	wildcard_resource: Option<Box<Resource>>,

	request_receiver: Option<BoxedService>,
	request_passer: Option<BoxedService>,
	request_handler: Option<BoxedService>,

	// TODO: configs, state, redirect, parent
	is_subtree_handler: bool,
}

// -------------------------

impl Resource {
	#[inline]
	pub fn is_subtree_handler(&self) -> bool {
		self.is_subtree_handler
	}

	#[inline]
	pub fn can_handle_request(&self) -> bool {
		self.request_handler.is_some()
	}
}

async fn request_receiver(mut request: Request) -> Result<Response, BoxedError> {
    let mut rs = request.extensions_mut().get_mut::<RoutingState>().unwrap();
    let cr = rs.current_resource.unwrap();

    if rs.path_segments.has_remaining_segments() {
        if cr.is_subtree_handler() {
            rs.subtree_handler_exists = true;
        }

        request = match cr.request_passer.as_ref() {
            Some(request_passer) => {
                let result = request_passer.clone_boxed().call(request).await;
                let Ok(mut response) = result else {
                    return result;
                };

                if response.status() != StatusCode::NOT_FOUND
                    || !cr.is_subtree_handler()
                    || !cr.can_handle_request()
                {
                    return Ok(response);
                }

                let Some(unused_request) = response.extensions_mut().remove::<UnusedRequest>()
                else {
                    return Ok(response);
                };

                unused_request.into_request()
            }
            None => {
                let result = request_passer(request).await;
                let Ok(mut response) = result else {
                    return result;
                };

                if response.status() != StatusCode::NOT_FOUND
                    || !cr.is_subtree_handler()
                    || !cr.can_handle_request()
                {
                    return Ok(response);
                }

                let Some(unused_request) = response.extensions_mut().remove::<UnusedRequest>()
                else {
                    return Ok(response);
                };

                unused_request.into_request()
            }
        };
    }

    request_handler(request).await
}

fn request_passer(mut req: Request) -> Pin<Box<dyn Future<Output = Result<Response, BoxedError>>>> {
    Box::pin(async move {
        let mut rs = req.extensions_mut().get_mut::<RoutingState>().unwrap();
        let cr = rs.current_resource.unwrap();
        let next_path_segment = rs.path_segments.next().unwrap();

        let some_next_resource = 'some_next_resource: {
            if let Some(next_resource) = cr.static_resources.as_ref().and_then(|static_resources| {
                match static_resources
                    .binary_search_by(|resource| resource.name.cmp(next_path_segment.as_str()))
                {
                    Ok(i) => Some(&static_resources[i]),
                    Err(_) => None,
                }
            }) {
                break 'some_next_resource Some(next_resource);
            }

            None
        };

        if let Some(next_resource) = some_next_resource {
            rs.current_resource.replace(next_resource);
            let result = request_receiver(req).await;
            let Ok(mut response) = result else {
                return result;
            };

            let Some(unused_request) = response.extensions_mut().remove::<UnusedRequest>() else {
                return Ok(response);
            };

            req = unused_request.into();

            rs = req.extensions_mut().get_mut::<RoutingState>().unwrap();
            rs.current_resource.replace(cr);
            rs.path_segments.revert_to_segment(next_path_segment);

            response.extensions_mut().insert(UnusedRequest::from(req));

            return Ok(response);
        }

        let mut response = Response::default();
        *response.status_mut() = StatusCode::NOT_FOUND;
        response.extensions_mut().insert(UnusedRequest::from(req));

        Ok(response)
    })
}

async fn request_handler(_req: Request) -> Result<Response, BoxedError> {
	todo!()
}
