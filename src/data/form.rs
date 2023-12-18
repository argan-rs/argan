use super::*;

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

// --------------------------------------------------
// Form

pub struct Form<T>(pub T);

impl<B, T> FromRequest<B> for Form<T>
where
	B: Body,
	B::Error: Debug,
	T: DeserializeOwned,
{
	type Error = Response;
	type Future = FormFuture<B, T>;

	fn from_request(request: Request<B>) -> Self::Future {
		FormFuture {
			request,
			_mark: PhantomData,
		}
	}
}

pin_project! {
	pub struct FormFuture<B, T> {
		#[pin] request: Request<B>,
		_mark: PhantomData<T>,
	}
}

impl<B, T> Future for FormFuture<B, T>
where
	B: Body,
	B::Error: Debug,
	T: DeserializeOwned,
{
	type Output = Result<Form<T>, Response>;

	fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
		let self_projection = self.project();

		let content_type = self_projection
			.request
			.headers()
			.get(CONTENT_TYPE)
			.unwrap()
			.to_str()
			.unwrap();

		if content_type == mime::APPLICATION_WWW_FORM_URLENCODED {
			if let Poll::Ready(result) = pin!(self_projection.request.collect()).poll(cx) {
				let body = result.unwrap().to_bytes();
				let value = serde_urlencoded::from_bytes::<T>(&body).unwrap();

				return Poll::Ready(Ok(Form(value)));
			}

			Poll::Pending
		} else {
			Poll::Ready(Err(StatusCode::BAD_REQUEST.into_response()))
		}
	}
}

impl<T> IntoResponse for Form<T>
where
	T: Serialize,
{
	fn into_response(self) -> Response {
		let form_string = serde_urlencoded::to_string(self.0).unwrap();
		let mut response = form_string.into_response();
		response.headers_mut().insert(
			CONTENT_TYPE,
			HeaderValue::from_static(mime::APPLICATION_WWW_FORM_URLENCODED.as_ref()),
		);

		response
	}
}
