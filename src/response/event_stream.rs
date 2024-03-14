use std::{
	pin::Pin,
	task::{Context, Poll},
	time::Duration,
};

use bytes::{BufMut, Bytes, BytesMut};
use futures_util::{Future, Stream};
use http::{
	header::{CACHE_CONTROL, CONTENT_TYPE},
	HeaderValue, StatusCode,
};
use http_body_util::BodyExt;
use pin_project::pin_project;
use serde::Serialize;

use crate::{
	body::{Body, Frame, HttpBody},
	common::{
		timer::{Interval, UninitializedTimer},
		BoxedError,
	},
	response::{IntoResponse, Response},
};

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

pub struct EventStream<S> {
	inner: S,
	keep_alive_interval: Option<Interval>,
}

impl<S> EventStream<S> {
	pub fn new(stream: S) -> Self {
		Self::try_to_create(stream).unwrap()
	}

	pub fn try_to_create(stream: S) -> Result<Self, EventStreamError> {
		let interval = Interval::try_new(Duration::from_secs(15))?;

		Ok(Self {
			inner: stream,
			keep_alive_interval: Some(interval),
		})
	}

	pub fn with_keep_alive_duration(mut self, some_duration: Option<Duration>) -> Self {
		if let Some(duration) = some_duration {
			self.keep_alive_interval = self
				.keep_alive_interval
				.take()
				.map(|mut interval| {
					interval.set_duration(duration);
					interval.reset();

					interval
				})
				.or_else(|| {
					Some(
						Interval::try_new(duration)
							.expect("a valid instance of EventStream should prove the TIMER was initialized"),
					)
				});

			return self;
		}

		self.keep_alive_interval = None;

		self
	}

	fn into_body(self) -> EventStreamBody<S> {
		EventStreamBody {
			inner: self.inner,
			keep_alive_interval: self.keep_alive_interval,
		}
	}
}

impl<S, E> IntoResponse for EventStream<S>
where
	S: Stream<Item = Result<Event, E>> + Send + Sync + 'static,
	E: Into<BoxedError> + Send + Sync,
{
	fn into_response(self) -> Response {
		let mut response = Response::new(Body::new(self.into_body()));
		response.headers_mut().insert(
			CONTENT_TYPE,
			HeaderValue::from_static(mime::TEXT_EVENT_STREAM.as_ref()),
		);

		response
			.headers_mut()
			.insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));

		response
	}
}

// -------------------------

#[pin_project]
struct EventStreamBody<S> {
	#[pin]
	inner: S,
	keep_alive_interval: Option<Interval>,
}

impl<S, E> HttpBody for EventStreamBody<S>
where
	S: Stream<Item = Result<Event, E>>,
	E: Into<BoxedError>,
{
	type Data = Bytes;
	type Error = BoxedError;

	fn poll_frame(
		self: Pin<&mut Self>,
		cx: &mut Context<'_>,
	) -> Poll<Option<Result<Frame<Bytes>, BoxedError>>> {
		let self_projection = self.project();

		match self_projection.inner.poll_next(cx) {
			Poll::Ready(None) => Poll::Ready(None),
			Poll::Ready(Some(result)) => match result {
				Ok(event) => {
					if let Some(interval) = self_projection.keep_alive_interval {
						interval.reset();
					}

					Poll::Ready(Some(Ok(Frame::data(event.into_bytes()))))
				}
				Err(error) => Poll::Ready(Some(Err(error.into()))),
			},
			Poll::Pending => {
				if let Some(interval) = self_projection.keep_alive_interval {
					interval
						.pin()
						.poll(cx)
						.map(|_| Some(Ok(Frame::data(Event::keep_alive()))))
				} else {
					Poll::Pending
				}
			}
		}
	}
}

// --------------------------------------------------

#[derive(Default)]
pub struct Event {
	buffer: BytesMut,
	flags: EventFlags,
}

impl Event {
	#[inline(always)]
	pub fn new() -> Self {
		Self::default()
	}

	#[inline]
	pub fn with_id<T: AsRef<str>>(mut self, id: T) -> Self {
		self.try_to_set_id(id).unwrap();

		self
	}

	pub fn try_to_set_id<T: AsRef<str>>(&mut self, id: T) -> Result<(), EventStreamError> {
		if self.flags.has(EventFlags::ID) {
			return Err(EventStreamError::CannotBeSetMultipleTimes("id"));
		}

		self.try_to_add_field("id", id.as_ref(), &['\r', '\n', '\0'])?;
		self.flags.add(EventFlags::ID);

		Ok(())
	}

	#[inline]
	pub fn with_type<T: AsRef<str>>(mut self, name: T) -> Self {
		self.try_to_set_type(name).unwrap();

		self
	}

	pub fn try_to_set_type<T: AsRef<str>>(&mut self, name: T) -> Result<(), EventStreamError> {
		if self.flags.has(EventFlags::NAME) {
			return Err(EventStreamError::CannotBeSetMultipleTimes("event"));
		}

		self.try_to_add_field("event", name.as_ref(), &['\r', '\n'])?;
		self.flags.add(EventFlags::NAME);

		Ok(())
	}

	#[inline]
	pub fn with_data<T: AsRef<str>>(mut self, data: T) -> Self {
		self.try_to_add_data(data).unwrap();

		self
	}

	pub fn try_to_add_data<T: AsRef<str>>(&mut self, data: T) -> Result<(), EventStreamError> {
		if self.flags.has(EventFlags::SERIALIZED_DATA) {
			return Err(EventStreamError::CannotCoexistWithSerializedData);
		}

		let value_str = data.as_ref();
		let mut value_chars = value_str.char_indices();

		let mut previous_char = ' ';
		let mut next_segment_index = 0;

		while let Some((i, ch)) = value_chars.next() {
			match ch {
				'\r' => {
					let value = &value_str[next_segment_index..i];
					self.try_to_add_field("data", value, &[])?;

					next_segment_index = i + 1;
				}
				'\n' => {
					if previous_char == '\r' {
						previous_char = ch;
						next_segment_index += 1;

						continue;
					}

					let value = &value_str[next_segment_index..i];
					self.try_to_add_field("data", value, &[])?;

					next_segment_index = i + 1;
				}
				_ => {}
			}

			previous_char = ch;
		}

		if next_segment_index < value_str.len() {
			let value = &value_str[next_segment_index..];
			self.try_to_add_field("data", value, &[])?;
		}

		self.flags.add(EventFlags::DATA);

		Ok(())
	}

	#[inline]
	pub fn with_json_data<T: Serialize>(mut self, json_data: T) -> Self {
		self.try_to_set_json_data(json_data).unwrap();

		self
	}

	pub fn try_to_set_json_data<T: Serialize>(
		&mut self,
		json_data: T,
	) -> Result<(), EventStreamError> {
		if self
			.flags
			.has_any(EventFlags::DATA | EventFlags::SERIALIZED_DATA)
		{
			return Err(EventStreamError::CannotCoexistWithExistingData);
		}

		let json = serde_json::to_string(&json_data)?;
		self
			.try_to_add_data(&json)
			.expect("shouldn't be called when DATA or SERIALIZED_DATA flags exist");

		self.flags.add(EventFlags::SERIALIZED_DATA);

		Ok(())
	}

	#[inline]
	pub fn with_retry(mut self, duration: Duration) -> Self {
		self.try_to_set_retry(duration).unwrap();

		self
	}

	pub fn try_to_set_retry(&mut self, duration: Duration) -> Result<(), EventStreamError> {
		if self.flags.has(EventFlags::RETRY) {
			return Err(EventStreamError::CannotBeSetMultipleTimes("retry"));
		}

		self.try_to_add_field("retry", &duration.as_millis().to_string(), &[])?;
		self.flags.add(EventFlags::RETRY);

		Ok(())
	}

	#[inline]
	pub fn with_comment<T: AsRef<str>>(mut self, comment: T) -> Self {
		self.try_to_add_comment(comment).unwrap();

		self
	}

	#[inline]
	pub fn try_to_add_comment<T: AsRef<str>>(&mut self, comment: T) -> Result<(), EventStreamError> {
		self.try_to_add_field("", comment.as_ref(), &['\r', '\n'])?;

		Ok(())
	}

	#[inline]
	fn try_to_add_field(
		&mut self,
		field: &'static str,
		value: &str,
		forbiddin_chars: &[char],
	) -> Result<(), EventStreamError> {
		if !forbiddin_chars.is_empty() && value.contains(forbiddin_chars) {
			return Err(EventStreamError::ForbiddenChars(
				if field.is_empty() { "comment" } else { field },
				forbiddin_chars.to_vec(),
			));
		}

		let field = field.as_bytes();
		let value = value.as_bytes();
		let size = field.len() + value.len() + 3; // 2 is for ":" and "\n", 1 is a reserve

		self.buffer.reserve(size);
		self.buffer.put_slice(field);
		self.buffer.put_u8(b':');
		self.buffer.put_slice(value);
		self.buffer.put_u8(b'\n');

		Ok(())
	}

	#[inline]
	pub(crate) fn into_bytes(mut self) -> Bytes {
		// We must have a one-byte reserve already, but...
		if self.buffer.capacity() == 0 {
			self.buffer.reserve(1);
		}

		self.buffer.put_u8(b'\n');

		self.buffer.freeze()
	}

	#[inline(always)]
	pub(crate) fn keep_alive() -> Bytes {
		let mut keep_alive_event = Event::default();
		keep_alive_event
			.try_to_add_comment("")
			.expect("empty comment must be valid");

		keep_alive_event.into_bytes()
	}
}

// ----------

bit_flags! {
	#[derive(Default)]
	EventFlags: u8 {
		NAME = 0b_0000_0001;
		DATA = 0b_0000_0010;
		SERIALIZED_DATA = 0b_0000_0100;
		ID = 0b_0000_1000;
		RETRY = 0b_0001_0000;
	}
}

// --------------------------------------------------
// EventStreamError

#[derive(Debug, crate::ImplError)]
pub enum EventStreamError {
	#[error(transparent)]
	UninitializedTimer(#[from] UninitializedTimer),
	#[error("'{0}' cannot be set multiple times")]
	CannotBeSetMultipleTimes(&'static str),
	#[error("cannot coexist with serialized data")]
	CannotCoexistWithSerializedData,
	#[error("cannot coexist with existing data")]
	CannotCoexistWithExistingData,
	#[error(transparent)]
	SerializationError(#[from] serde_json::Error),
	#[error("'{0}' field value cannot contain any of these {:?} characters", .1.as_slice())]
	ForbiddenChars(&'static str, Vec<char>),
}

impl IntoResponse for EventStreamError {
	fn into_response(self) -> Response {
		StatusCode::INTERNAL_SERVER_ERROR.into_response()
	}
}

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

#[cfg(test)]
mod test {
	use core::panic;

	use crate::common::timer::set_timer;

	use super::*;

	// --------------------------------------------------------------------------------
	// --------------------------------------------------------------------------------

	#[derive(Debug, Serialize)]
	struct Data {
		name: &'static str,
		message: &'static str,
	}

	#[test]
	fn event() {
		// ----------

		let event = Event::new()
			.with_type("e")
			.with_id("42")
			.with_data("data 1")
			.with_data("data 2\ndata 3")
			.with_comment("comment 1")
			.with_comment("comment 2")
			.with_retry(Duration::from_millis(10))
			.into_bytes();

		assert_eq!(
			"event:e\nid:42\ndata:data 1\ndata:data 2\ndata:data 3\n:comment 1\n:comment 2\nretry:10\n\n",
			event,
		);

		// ----------

		let data = Data {
			name: "R2D2",
			message: "message",
		};

		let event = Event::new()
			.with_id("42")
			.with_comment("comment")
			.with_json_data(&data)
			.with_retry(Duration::from_secs(10))
			.into_bytes();

		assert_eq!(
			"id:42\n:comment\ndata:{\"name\":\"R2D2\",\"message\":\"message\"}\nretry:10000\n\n",
			event,
		);

		// -------------------------
		// id

		let mut event = Event::new().with_type("test").with_id("42");
		match event.try_to_set_id("42").unwrap_err() {
			EventStreamError::CannotBeSetMultipleTimes("id") => {}
			error => panic!("unexpected error: {}", error),
		}

		// ----------

		let mut event = Event::new().with_type("test").with_id("42");
		match event.try_to_set_id("").unwrap_err() {
			EventStreamError::CannotBeSetMultipleTimes("id") => {}
			error => panic!("unexpected error: {}", error),
		}

		// ----------

		let mut error = Event::new()
			.with_type("test")
			.try_to_set_id("4\r2")
			.unwrap_err();
		match error {
			EventStreamError::ForbiddenChars("id", forbiddin_chars) => {
				assert_eq!(forbiddin_chars, ['\r', '\n', '\0']);
			}
			error => panic!("unexpected error: {}", error),
		}

		// ----------

		let mut error = Event::new()
			.with_type("test")
			.try_to_set_id("4\n2")
			.unwrap_err();
		match error {
			EventStreamError::ForbiddenChars("id", forbiddin_chars) => {
				assert_eq!(forbiddin_chars, ['\r', '\n', '\0']);
			}
			error => panic!("unexpected error: {}", error),
		}

		// ----------

		let mut error = Event::new()
			.with_type("test")
			.try_to_set_id("4\02")
			.unwrap_err();
		match error {
			EventStreamError::ForbiddenChars("id", forbiddin_chars) => {
				assert_eq!(forbiddin_chars, ['\r', '\n', '\0']);
			}
			error => panic!("unexpected error: {}", error),
		}

		// -------------------------
		// event type

		let mut event = Event::new().with_type("test");
		match event.try_to_set_type("type").unwrap_err() {
			EventStreamError::CannotBeSetMultipleTimes("event") => {}
			error => panic!("unexpected error: {}", error),
		}

		// ----------

		let mut error = Event::new().try_to_set_type("test\r").unwrap_err();
		match error {
			EventStreamError::ForbiddenChars("event", forbidden_chars) => {
				assert_eq!(forbidden_chars, ['\r', '\n']);
			}
			error => panic!("unexpected error: {}", error),
		}

		// ----------

		let mut error = Event::new().try_to_set_type("test\n").unwrap_err();
		match error {
			EventStreamError::ForbiddenChars("event", forbidden_chars) => {
				assert_eq!(forbidden_chars, ['\r', '\n']);
			}
			error => panic!("unexpected error: {}", error),
		}

		// -------------------------
		// data + json data

		let mut event = Event::new().with_id("42").with_data("data");
		match event.try_to_set_json_data(&data).unwrap_err() {
			EventStreamError::CannotCoexistWithExistingData => {}
			error => panic!("unexpected error: {}", error),
		}

		// -------------------------
		// json data + data

		let mut event = Event::new()
			.with_retry(Duration::from_secs(5))
			.with_json_data(&data);

		match event.try_to_add_data("data").unwrap_err() {
			EventStreamError::CannotCoexistWithSerializedData => {}
			error => panic!("unexpected error: {}", error),
		}

		// -------------------------
		// retry

		let mut event = Event::new()
			.with_type("test")
			.with_retry(Duration::from_secs(5));

		match event.try_to_set_retry(Duration::from_secs(10)).unwrap_err() {
			EventStreamError::CannotBeSetMultipleTimes("retry") => {}
			error => panic!("unexpected error: {}", error),
		}

		// -------------------------
		// comment

		let mut error = Event::new()
			.with_id("42")
			.try_to_add_comment("\rcomment")
			.unwrap_err();
		match error {
			EventStreamError::ForbiddenChars("comment", forbidden_chars) => {
				assert_eq!(forbidden_chars, ['\r', '\n']);
			}
			error => panic!("unexpected error: {}", error),
		}

		// ----------

		let mut error = Event::new()
			.with_id("42")
			.try_to_add_comment("\ncomment")
			.unwrap_err();

		match error {
			EventStreamError::ForbiddenChars("comment", forbidden_chars) => {
				assert_eq!(forbidden_chars, ['\r', '\n']);
			}
			error => panic!("unexpected error: {}", error),
		}
	}

	#[tokio::test]
	async fn event_stream() {
		use futures_util::stream::unfold;
		use hyper_util::rt::TokioTimer;

		// --------------------------------------------------------------------------------
		// --------------------------------------------------------------------------------

		set_timer(TokioTimer::new());

		let data = &Data {
			name: "C-3PO",
			message: "message",
		};

		// -------------------------

		let stream = unfold(0, move |mut number| async move {
			if number == 0 {
				tokio::time::sleep(Duration::from_secs(1)).await;

				let event = Event::new()
					.with_type("test")
					.with_id("42")
					.with_data("data 1\ndata 2");

				return Some((Result::<_, EventStreamError>::Ok(event), number + 1));
			}

			if number == 1 {
				tokio::time::sleep(Duration::from_secs(1)).await;

				let event = Event::new()
					.with_id("42")
					.with_json_data(&data)
					.with_comment("...")
					.with_retry(Duration::from_secs(1));

				return Some((Ok(event), number + 1));
			}

			None
		});

		let stream_response = EventStream::new(stream)
			.with_keep_alive_duration(Some(Duration::from_millis(750)))
			.into_response();

		assert_eq!(
			mime::TEXT_EVENT_STREAM.as_ref(),
			stream_response
				.headers()
				.get(CONTENT_TYPE)
				.unwrap()
				.to_str()
				.unwrap()
		);

		assert_eq!(
			"no-store",
			stream_response
				.headers()
				.get(CACHE_CONTROL)
				.unwrap()
				.to_str()
				.unwrap()
		);

		let now = std::time::Instant::now();

		let mut body = stream_response.into_body();
		let keep_alive = body.frame().await.unwrap().unwrap().into_data().unwrap();
		assert_eq!(keep_alive, ":\n\n");

		let elapsed = now.elapsed();
		dbg!(&elapsed);
		if elapsed < Duration::from_millis(750) {
			panic!("keep alive came early");
		}

		let event_1 = body.frame().await.unwrap().unwrap().into_data().unwrap();
		assert_eq!(event_1, "event:test\nid:42\ndata:data 1\ndata:data 2\n\n");

		let elapsed = now.elapsed();
		dbg!(&elapsed);
		if elapsed < Duration::from_secs(1) {
			panic!("event 1 came early");
		}

		let keep_alive = body.frame().await.unwrap().unwrap().into_data().unwrap();
		assert_eq!(keep_alive, ":\n\n");

		let elapsed = now.elapsed();
		dbg!(&elapsed);
		if elapsed < Duration::from_millis(1750) {
			panic!("keep alive came early");
		}

		let event_2 = body.frame().await.unwrap().unwrap().into_data().unwrap();
		assert_eq!(
			event_2,
			"id:42\ndata:{\"name\":\"C-3PO\",\"message\":\"message\"}\n:...\nretry:1000\n\n"
		);

		let elapsed = now.elapsed();
		dbg!(&elapsed);
		if elapsed < Duration::from_secs(2) {
			panic!("event 1 came early");
		}
	}
}
