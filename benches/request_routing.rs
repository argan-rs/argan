use bytes::Bytes;
use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};

use http::StatusCode;
use http_body_util::Empty;
use hyper::service::Service as HyperService;
use tokio::runtime::Builder;

// ----------

use argan::{
	handler::_get,
	request::{Request, RequestContext},
	resource::Resource,
};

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

pub fn request_routing(c: &mut Criterion) {
	struct Param {
		static_patterns: [&'static str; 5],
		regex_patterns: [(&'static str, &'static str); 5],
		wildcard_pattern: &'static str,
	}

	let param = Param {
		static_patterns: ["/login", "/logout", "/about", "/products", "/categories"],
		regex_patterns: [
			("/{year", r":\d{4}-\d{2}-\d{2}}"),
			("/{news", ":foreign|domestic|sports}"),
			("/{forecast_days", ":5|10}_days_forecast"),
			("/{brand", r":[^\[]+}[001]"),
			("/id: {prefix", ":[A-Z]{3}}"),
		],
		wildcard_pattern: "language",
	};

	// Last static resource will have a handler and subresources.
	fn add_static_resources(resource: &mut Resource, params: (u8, &Param)) {
		let handler = |_request: RequestContext| async {};
		// println!("\nsegment index: {}", params.0);
		let next_segment_index = params.0 + 1;

		// -----

		params.1.static_patterns.iter().for_each(|pattern| {
			// println!("static pattern: {}", pattern);
			resource.subresource_mut(pattern);
		});

		if params.0 < 10 {
			let last_static_resource = resource.subresource_mut(params.1.static_patterns.last().unwrap());
			last_static_resource.set_handler_for(_get(handler));
			add_static_resources(last_static_resource, (next_segment_index, params.1));
		}

		// -----

		// params.1.regex_patterns.iter().for_each(|(name, pattern)| {
		// 	let pattern = format!("{}{}{}", name, next_segment_index, pattern);
		// 	// println!("regex pattern: {}", pattern);
		// 	resource.subresource_mut(&pattern);
		// });

		// -----

		// let pattern = format!("{}{}", params.1.wildcard_pattern, next_segment_index);
		// // println!("wildcard pattern: {}", pattern);
		// resource.subresource_mut(&pattern);
	}

	// Last regex resource will have a handler and subresources.
	fn add_regex_resources(resource: &mut Resource, params: (u8, &Param)) {
		let handler = |_request: RequestContext| async {};
		// println!("\nsegment index: {}", params.0);
		let next_segment_index = params.0 + 1;

		// -----

		// params.1.static_patterns.iter().for_each(|pattern| {
		// 	// println!("static pattern: {}", pattern);
		// 	resource.subresource_mut(pattern);
		// });

		// -----

		params
			.1
			.regex_patterns
			.iter()
			.for_each(|(capture_name, subpattern)| {
				let pattern = format!("{}{}{}", capture_name, next_segment_index, subpattern);
				// dbg!(&pattern);
				resource.subresource_mut(&pattern);
			});

		if params.0 < 10 {
			let (capture_name, subpattern) = params.1.regex_patterns.last().unwrap();
			let last_regex_resource = resource.subresource_mut(&format!(
				"{}{}{}",
				capture_name, next_segment_index, subpattern
			));
			last_regex_resource.set_handler_for(_get(handler));
			add_regex_resources(last_regex_resource, (next_segment_index, params.1));
		}

		// -----

		// let pattern = format!("{}{}", params.1.wildcard_pattern, next_segment_index);
		// // println!("wildcard pattern: {}", pattern);
		// resource.subresource_mut(&pattern);
	}

	// Each wildcard resource will have a handler and subresources.
	fn add_wildcard_resources(resource: &mut Resource, params: (u8, &Param)) {
		let handler = |_request: RequestContext| async {};
		// println!("\nsegment index: {}", params.0);
		let next_segment_index = params.0 + 1;

		// -----

		// params.1.static_patterns.iter().for_each(|pattern| {
		// 	// println!("static pattern: {}", pattern);
		// 	resource.subresource_mut(pattern);
		// });

		// -----

		// params.1.regex_patterns.iter().for_each(|(name, pattern)| {
		// 	let pattern = format!("{}{}{}", name, next_segment_index, pattern);
		// 	// println!("regex pattern: {}", pattern);
		// 	resource.subresource_mut(&pattern);
		// });

		// -----

		let pattern = format!("/{{{}{}}}", params.1.wildcard_pattern, next_segment_index);
		// println!("wildcard pattern: {}", pattern);
		let wildcard_resource = resource.subresource_mut(&pattern);
		wildcard_resource.set_handler_for(_get(handler));

		if params.0 < 10 {
			// println!("calling for subresources of {}", resource.pattern());
			add_wildcard_resources(wildcard_resource, (next_segment_index, params.1));
		}
	}

	// -------------------------

	let runtime = Builder::new_multi_thread()
		.worker_threads(1)
		.build()
		.unwrap();

	let mut bench_group = c.benchmark_group("request_routing");
	bench_group.sample_size(1000);

	// -------------------------
	// static resources

	let mut root = Resource::new("/");
	add_static_resources(&mut root, (0, &param));

	let service = root.into_service();

	bench_group.bench_function(BenchmarkId::new("static segments", 1), |b| {
		b.to_async(&runtime).iter(|| async {
			let request = Request::get("/categories")
				.body(Empty::<Bytes>::new())
				.unwrap();
			let response = service.call(request).await.unwrap();
			assert_eq!(response.status(), StatusCode::OK);
		})
	});

	bench_group.bench_function(BenchmarkId::new("static segments", 5), |b| {
		b.to_async(&runtime).iter(|| async {
			let request = Request::get("/categories/categories/categories/categories/categories")
				.body(Empty::<Bytes>::new())
				.unwrap();

			let response = service.call(request).await.unwrap();
			assert_eq!(response.status(), StatusCode::OK);
		})
	});

	bench_group.bench_function(BenchmarkId::new("static segments", 10), |b| {
		b.to_async(&runtime).iter(
			|| async {
				let request = Request::get(
					"/categories/categories/categories/categories/categories/categories/categories/categories/categories/categories",
				).body(Empty::<Bytes>::new()).unwrap();

				let response = service.call(request).await.unwrap();
				assert_eq!(response.status(), StatusCode::OK);
			},
		)
	});

	// -------------------------
	// regex resources

	let mut root = Resource::new("/");
	add_regex_resources(&mut root, (0, &param));

	let service = root.into_service();

	bench_group.bench_function(BenchmarkId::new("regex segments", 1), |b| {
		b.to_async(&runtime).iter(|| async {
			let request = Request::get("/id:%20ABC")
				.body(Empty::<Bytes>::new())
				.unwrap();
			let response = service.call(request).await.unwrap();
			assert_eq!(response.status(), StatusCode::OK);
		})
	});

	bench_group.bench_function(BenchmarkId::new("regex segments", 5), |b| {
		b.to_async(&runtime).iter(|| async {
			let request = Request::get("/id:%20ABC/id:%20ABC/id:%20ABC/id:%20ABC/id:%20ABC")
				.body(Empty::<Bytes>::new())
				.unwrap();

			let response = service.call(request).await.unwrap();
			assert_eq!(response.status(), StatusCode::OK);
		})
	});

	bench_group.bench_function(BenchmarkId::new("regex segments", 10), |b| {
		b.to_async(&runtime).iter(
			|| async {
				let request = Request::get(
					"/id:%20ABC/id:%20ABC/id:%20ABC/id:%20ABC/id:%20ABC/id:%20ABC/id:%20ABC/id:%20ABC/id:%20ABC/id:%20ABC",
				).body(Empty::<Bytes>::new()).unwrap();

				let response = service.call(request).await.unwrap();
				assert_eq!(response.status(), StatusCode::OK);
			},
		)
	});

	// -------------------------
	// wildcard resources

	let mut root = Resource::new("/");
	add_wildcard_resources(&mut root, (0, &param));

	let service = root.into_service();

	bench_group.bench_function(BenchmarkId::new("wildcard segments", 1), |b| {
		b.to_async(&runtime).iter(|| async {
			let request = Request::get("/wildcard")
				.body(Empty::<Bytes>::new())
				.unwrap();
			let response = service.call(request).await.unwrap();
			assert_eq!(response.status(), StatusCode::OK);
		})
	});

	bench_group.bench_function(BenchmarkId::new("wildcard segments", 5), |b| {
		b.to_async(&runtime).iter(|| async {
			let request = Request::get("/wildcard/wildcard/wildcard/wildcard/wildcard")
				.body(Empty::<Bytes>::new())
				.unwrap();

			let response = service.call(request).await.unwrap();
			assert_eq!(response.status(), StatusCode::OK);
		})
	});

	bench_group.bench_function(BenchmarkId::new("wildcard segments", 10), |b| {
		b.to_async(&runtime).iter(|| async {
			let request = Request::get(
					"/wildcard/wildcard/wildcard/wildcard/wildcard/wildcard/wildcard/wildcard/wildcard/wildcard",
				).body(Empty::<Bytes>::new()).unwrap();

			let response = service.call(request).await.unwrap();
			assert_eq!(response.status(), StatusCode::OK);
		})
	});

	bench_group.finish();
}

criterion_group!(benches, request_routing);
criterion_main!(benches);
