use std::convert::Infallible;

use bytes::Bytes;
use http::{
	header::{CONTENT_TYPE, LOCATION},
	StatusCode,
};
use http_body_util::{BodyExt, Empty};
use hyper::service::Service;
use serde::{Deserialize, Serialize};

use crate::{
	data::json::Json,
	handler::{_get, _post, _wildcard_method},
	pattern::DeserializerError,
	request::{PathParamsError, Request, RequestHead},
	resource::Resource,
	response::{IntoResponseResult, Response},
};

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

// --------------------------------------------------------------------------------
// Service Test Helpers

// --------------------------------------------------
// Structs for deserialization

#[allow(non_camel_case_types)]
#[derive(Debug, Default, Serialize, Deserialize, PartialEq)]
pub(crate) struct Rx_2_0 {
	pub(crate) sub: Option<String>,
	pub(crate) wl_1_0: u32,
	pub(crate) rx_2_0: String,
}

#[allow(non_camel_case_types)]
#[derive(Debug, Default, Serialize, Deserialize, PartialEq)]
pub(crate) struct Wl_3_0 {
	pub(crate) sub: Option<String>,
	pub(crate) wl_1_0: u32,
	pub(crate) rx_2_1: String,
	pub(crate) wl_3_0: bool,
}

#[allow(non_camel_case_types)]
#[derive(Debug, Default, Serialize, Deserialize, PartialEq)]
pub(crate) struct Rx_1_1 {
	pub(crate) sub: Option<String>,
	pub(crate) rx_1_1: String,
}

#[allow(non_camel_case_types)]
#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub(crate) enum DataKind {
	None,
	String(String),
	Rx_2_0(Rx_2_0),
	Wl_3_0(Wl_3_0),
	Rx_1_1(Rx_1_1),
}

// --------------------------------------------------
// Case

pub(crate) struct Case {
	pub(crate) name: &'static str,
	pub(crate) method: &'static str,
	pub(crate) host: &'static str,
	pub(crate) path: &'static str,
	pub(crate) some_content_type: Option<mime::Mime>,
	pub(crate) some_redirect_location: Option<&'static str>,
	pub(crate) data_kind: DataKind,
}

// --------------------------------------------------
// Dummy Handler

async fn dummy_handler() {}

// --------------------------------------------------
// new_root()

pub(crate) fn new_root() -> Resource {
	//	/	->	/st_0_0	->	/{wl_1_0}	->	/{rx_2_0:p_0}/
	//							|							|	->	/{rx_2_1:p_1}-abc	->	/{wl_3_0}
	//							|
	//							|	->	/{rx_1_1:p_0}-abc/	->	/st_2_0
	//																			|	->	/st_2_1

	let mut root = Resource::new("/");
	root.set_handler_for(_get.to(|head: RequestHead| async move {
		let result = head.path_params_as::<String>();

		dbg!(&result);

		match result {
			Ok(data) => Json(data).into_response_result(),
			Err(error) => {
				let PathParamsError(DeserializerError::NoDataIsAvailable) = error else {
					panic!("unexpected error: {}", error);
				};

				"Hello, World!".into_response_result()
			}
		}
	}));

	root
		.subresource_mut("/st_0_0")
		.set_handler_for(_get.to(dummy_handler));

	root
		.subresource_mut("/st_0_0/{wl_1_0}/{rx_2_0:p_0}/")
		.set_handler_for(_get.to(|head: RequestHead| async move {
			let data = head.path_params_as::<Rx_2_0>().unwrap();

			Json(data)
		}));

	root
		.subresource_mut("/st_0_0/{wl_1_0}/{rx_2_1:p_1}-abc/{wl_3_0}")
		.set_handler_for(_post.to(|head: RequestHead| async move {
			let data = head.path_params_as::<Wl_3_0>().unwrap();

			Json(data)
		}));

	root
		.subresource_mut("/st_0_0/{rx_1_1:p_0}-abc/st_2_0")
		.set_handler_for(_get.to(|head: RequestHead| async move {
			let data = head.path_params_as::<Rx_1_1>().unwrap();

			Json(data)
		}));

	root
		.subresource_mut("/st_0_0/{rx_1_1:p_0}-abc/")
		.set_handler_for(_wildcard_method.to(Some(|head: RequestHead| async move {
			let data = head.path_params_as::<Rx_1_1>().unwrap();

			Json(data)
		})));

	root
		.subresource_mut("/st_0_0/{rx_1_1:p_0}-abc/st_2_1")
		.set_handler_for(_get.to(|| async { "Hello, World!" }));

	root
}

// -------------------------
// test_service()

pub(crate) async fn test_service<S>(service: S, cases: &[Case])
where
	S: Service<Request<Empty<Bytes>>, Response = Response, Error = Infallible>,
{
	//	router	->	host	->	/	->	/st_0_0	->	/{wl_1_0}	->	/{rx_2_0:p_0}/
	//																		|							|	->	/{rx_2_1:p_1}-abc	->	/{wl_3_0}
	//																		|
	//																		|	->	/{rx_1_1:p_0}-abc/	->	/st_2_0
	//																														|	->	/st_2_1

	let text_plain = mime::TEXT_PLAIN_UTF_8.as_ref();
	let application_json = mime::APPLICATION_JSON.as_ref();

	for case in cases {
		dbg!(case.name);

		let request = Request::builder()
			.method(case.method)
			.uri(case.host.to_string() + case.path)
			.body(Empty::default())
			.unwrap();

		let response = service.call(request).await.unwrap();

		if let Some(expected_location) = case.some_redirect_location {
			assert_eq!(response.status(), StatusCode::PERMANENT_REDIRECT);

			let location = response.headers().get(LOCATION).unwrap().to_str().unwrap();
			assert_eq!(location, expected_location);
			assert_eq!(
				response
					.into_body()
					.collect()
					.await
					.unwrap()
					.to_bytes()
					.len(),
				0
			);
		} else if let Some(expected_content_type) = case.some_content_type.as_ref() {
			assert_eq!(response.status(), StatusCode::OK);

			let content_type = response
				.headers()
				.get(CONTENT_TYPE)
				.unwrap()
				.to_str()
				.unwrap();

			assert_eq!(content_type, expected_content_type.as_ref());

			if content_type == text_plain {
				let DataKind::String(expected_data) = &case.data_kind else {
					unreachable!()
				};

				let data = response.into_body().collect().await.unwrap().to_bytes();
				assert_eq!(data, expected_data);
			} else if content_type == application_json {
				let json_body = response.into_body().collect().await.unwrap().to_bytes();
				match &case.data_kind {
					DataKind::None => {}
					DataKind::String(expected_data) => {
						let data = serde_json::from_slice::<String>(&json_body).unwrap();
						assert_eq!(data, *expected_data);
					}
					DataKind::Rx_2_0(expected_data) => {
						let data = serde_json::from_slice::<Rx_2_0>(&json_body).unwrap();
						assert_eq!(&data, expected_data);
					}
					DataKind::Wl_3_0(expected_data) => {
						let data = serde_json::from_slice::<Wl_3_0>(&json_body).unwrap();
						assert_eq!(&data, expected_data);
					}
					DataKind::Rx_1_1(expected_data) => {
						let data = serde_json::from_slice::<Rx_1_1>(&json_body).unwrap();
						assert_eq!(&data, expected_data);
					}
				}
			} else {
				unreachable!();
			}
		} else {
			assert_eq!(response.status(), StatusCode::OK);
		}
	}
}

// --------------------------------------------------------------------------------
