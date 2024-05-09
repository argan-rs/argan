//! Server-sent events

// ----------

use std::{
	pin::Pin,
	task::{Context, Poll},
	time::{Duration, Instant},
};

use argan_core::{
	body::{Body, Frame, HttpBody},
	BoxedError,
};
use bytes::{BufMut, Bytes, BytesMut};
use futures_util::Stream;
use http::{
	header::{CACHE_CONTROL, CONTENT_TYPE},
	HeaderValue,
};
use pin_project::pin_project;
use tokio::time::{interval_at, Interval, MissedTickBehavior};

#[cfg(feature = "json")]
use http::StatusCode;

#[cfg(feature = "json")]
use serde::Serialize;

use crate::response::{IntoResponse, Response};

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

/// Response type to stream server-sent events.
pub struct EventStream<S> {
	inner: S,
	keep_alive_interval: Option<Interval>,
}

impl<S> EventStream<S>
where
	S: Stream<Item = Event> + Send + Sync + 'static,
{
	/// Creates an event stream response.
	pub fn new(stream: S) -> Self {
		let duration = Duration::from_secs(15);
		let start_time = Instant::now() + duration;

		let mut interval = interval_at(start_time.into(), duration);
		interval.set_missed_tick_behavior(MissedTickBehavior::Delay);

		Self {
			inner: stream,
			keep_alive_interval: Some(interval),
		}
	}

	/// Sets the interval between keep-alive messages. The default is 15 seconds.
	///
	/// None turns off the keep-alive messages.
	pub fn with_keep_alive_interval(mut self, some_duration: Option<Duration>) -> Self {
		if let Some(duration) = some_duration {
			let start_time = Instant::now() + duration;
			let mut interval = interval_at(start_time.into(), duration);

			interval.set_missed_tick_behavior(MissedTickBehavior::Delay);

			self.keep_alive_interval = Some(interval);

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

impl<S> IntoResponse for EventStream<S>
where
	S: Stream<Item = Event> + Send + Sync + 'static,
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

impl<S> HttpBody for EventStreamBody<S>
where
	S: Stream<Item = Event>,
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
			Poll::Ready(Some(event)) => {
				if let Some(interval) = self_projection.keep_alive_interval {
					interval.reset();
				}

				Poll::Ready(Some(Ok(Frame::data(event.into_bytes()))))
			}
			Poll::Pending => {
				if let Some(interval) = self_projection.keep_alive_interval {
					interval
						.poll_tick(cx)
						.map(|_| Some(Ok(Frame::data(Event::keep_alive()))))
				} else {
					Poll::Pending
				}
			}
		}
	}
}

// --------------------------------------------------

/// Server-sent event.
#[derive(Default)]
pub struct Event {
	buffer: BytesMut,
	flags: EventFlags,
}

impl Event {
	/// Creates an empty event.
	#[inline(always)]
	pub fn new() -> Self {
		Self::default()
	}

	/// Sets the `id` field of the event.
	///
	/// # Panics
	/// - if called multiple times
	/// - if `id` contains '\r' (CR), '\n' (LF), or '\0' (NULL)
	pub fn with_id<T: AsRef<str>>(mut self, id: T) -> Self {
		if self.flags.has(EventFlags::ID) {
			panic!("'id' cannot be set multiple times");
		}

		self.add_field("id", id.as_ref(), &['\r', '\n', '\0']);
		self.flags.add(EventFlags::ID);

		self
	}

	/// Sets the `event` field of the event.
	///
	/// # Panics
	/// - if called multiple times
	/// - if `name` contains '\r' (CR), '\n' (LF)
	pub fn with_name<T: AsRef<str>>(mut self, name: T) -> Self {
		if self.flags.has(EventFlags::NAME) {
			panic!("'event' cannot be set multiple times")
		}

		self.add_field("event", name.as_ref(), &['\r', '\n']);
		self.flags.add(EventFlags::NAME);

		self
	}

	/// Adds a `data` field to the event.
	///
	/// For each '\r' (CR), '\n' (LF), or "\r\n" (CRLF), a new `data` field will be added
	/// with the content following them.
	///
	/// # Panics
	/// - if the event already contains JSON data
	pub fn with_data<T: AsRef<str>>(mut self, data: T) -> Self {
		if self.flags.has(EventFlags::SERIALIZED_DATA) {
			panic!("'data' cannot coexist with serialized data")
		}

		let value_str = data.as_ref();
		let value_chars = value_str.char_indices();

		let mut previous_char = ' ';
		let mut next_segment_index = 0;

		for (i, ch) in value_chars {
			match ch {
				'\r' => {
					let value = &value_str[next_segment_index..i];
					self.add_field("data", value, &[]);

					next_segment_index = i + 1;
				}
				'\n' => {
					if previous_char == '\r' {
						previous_char = ch;
						next_segment_index += 1;

						continue;
					}

					let value = &value_str[next_segment_index..i];
					self.add_field("data", value, &[]);

					next_segment_index = i + 1;
				}
				_ => {}
			}

			previous_char = ch;
		}

		if next_segment_index < value_str.len() {
			let value = &value_str[next_segment_index..];
			self.add_field("data", value, &[]);
		}

		self.flags.add(EventFlags::DATA);

		self
	}

	/// Sets the `data` field of the event to the JSON data.
	///
	/// # Panics
	/// - if the event already contains any data
	#[cfg(feature = "json")]
	pub fn with_json_data<T: Serialize>(mut self, json_data: T) -> Result<Self, EventError> {
		if self
			.flags
			.has_any(EventFlags::DATA | EventFlags::SERIALIZED_DATA)
		{
			panic!("serialized data cannot coexist with existing data")
		}

		let json = serde_json::to_string(&json_data)?;
		self = self.with_data(json);

		self.flags.add(EventFlags::SERIALIZED_DATA);

		Ok(self)
	}

	/// Sets the `retry` field of the event.
	///
	/// # Panics
	/// - if called multiple times
	pub fn with_retry(mut self, duration: Duration) -> Self {
		if self.flags.has(EventFlags::RETRY) {
			panic!("'retry' cannot be set multiple times")
		}

		self.add_field("retry", &duration.as_millis().to_string(), &[]);
		self.flags.add(EventFlags::RETRY);

		self
	}

	/// Adds a comment to the event.
	///
	/// # Panics
	/// - if `comment` contains '\r' (CR), '\n' (LF)
	pub fn with_comment<T: AsRef<str>>(mut self, comment: T) -> Self {
		self.add_field("", comment.as_ref(), &['\r', '\n']);

		self
	}

	fn add_field(&mut self, field: &'static str, value: &str, forbiddin_chars: &[char]) {
		if !forbiddin_chars.is_empty() && value.contains(forbiddin_chars) {
			panic!(
				"'{}' field value cannot contain any of these {:?} characters",
				if field.is_empty() { "comment" } else { field },
				forbiddin_chars.to_vec(),
			);
		}

		let field = field.as_bytes();
		let value = value.as_bytes();
		let size = field.len() + value.len() + 3; // 2 is for ":" and "\n", 1 is a reserve

		self.buffer.reserve(size);
		self.buffer.put_slice(field);
		self.buffer.put_u8(b':');
		self.buffer.put_slice(value);
		self.buffer.put_u8(b'\n');
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
		Event::default().with_comment("").into_bytes()
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
// EventStream

/// Returned when serialization of the JSON data fails.
#[cfg(feature = "json")]
#[derive(Debug, crate::ImplError)]
#[error(transparent)]
pub struct EventError(#[from] serde_json::Error);

#[cfg(feature = "json")]
impl IntoResponse for EventError {
	fn into_response(self) -> Response {
		StatusCode::INTERNAL_SERVER_ERROR.into_response()
	}
}

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

#[cfg(all(test, feature = "full"))]
mod test {
	#![allow(clippy::octal_escapes)]
	use core::panic;

	use http_body_util::BodyExt;

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

		let event_bytes = Event::new()
			.with_name("e")
			.with_id("42")
			.with_data("data 1")
			.with_data("data 2\ndata 3")
			.with_comment("comment 1")
			.with_comment("comment 2")
			.with_retry(Duration::from_millis(10))
			.into_bytes();

		assert_eq!(
			"event:e\nid:42\ndata:data 1\ndata:data 2\ndata:data 3\n:comment 1\n:comment 2\nretry:10\n\n",
			event_bytes,
		);

		// ----------

		let data = Data {
			name: "R2D2",
			message: "message",
		};

		let event_bytes = Event::new()
			.with_id("42")
			.with_comment("comment")
			.with_json_data(data)
			.unwrap()
			.with_retry(Duration::from_secs(10))
			.into_bytes();

		assert_eq!(
			"id:42\n:comment\ndata:{\"name\":\"R2D2\",\"message\":\"message\"}\nretry:10000\n\n",
			event_bytes,
		);
	}

	// -------------------------
	// id

	#[test]
	#[should_panic = "'id' cannot be set multiple times"]
	fn multiple_ids_1() {
		let _ = Event::new().with_name("test").with_id("42").with_id("42");
	}

	#[test]
	#[should_panic = "'id' cannot be set multiple times"]
	fn multiple_ids_2() {
		let _ = Event::new().with_name("test").with_id("42").with_id("");
	}

	#[test]
	#[should_panic = "'id' field value cannot contain any of these ['\\r', '\\n', '\\0'] characters"]
	fn id_forbidden_chars_1() {
		let _ = Event::new().with_name("test").with_id("4\r2");
	}

	#[test]
	#[should_panic = "'id' field value cannot contain any of these ['\\r', '\\n', '\\0'] characters"]
	fn id_forbidden_chars_2() {
		let _ = Event::new().with_name("test").with_id("4\n2");
	}

	#[test]
	#[should_panic = "'id' field value cannot contain any of these ['\\r', '\\n', '\\0'] characters"]
	fn id_forbidden_chars_3() {
		let _ = Event::new().with_name("test").with_id("4\02");
	}

	// -------------------------
	// event type

	#[test]
	#[should_panic = "'event' cannot be set multiple times"]
	fn multiple_events() {
		let _ = Event::new().with_name("test").with_name("type");
	}

	#[test]
	#[should_panic = "'event' field value cannot contain any of these ['\\r', '\\n'] characters"]
	fn event_forbidden_chars_1() {
		let _ = Event::new().with_name("test\r").with_id("4\02");
	}

	#[test]
	#[should_panic = "'event' field value cannot contain any of these ['\\r', '\\n'] characters"]
	fn event_forbidden_chars_2() {
		let _ = Event::new().with_name("test\n").with_id("4\02");
	}

	// -------------------------
	// data + JSON

	#[test]
	#[should_panic = "serialized data cannot coexist with existing data"]
	fn data_and_json() {
		let data = Data {
			name: "R2D2",
			message: "message",
		};

		let _ = Event::new()
			.with_id("42")
			.with_data("data")
			.with_json_data(data);
	}

	// -------------------------
	// JSON + data

	#[test]
	#[should_panic = "'data' cannot coexist with serialized data"]
	fn json_and_data() {
		let data = Data {
			name: "R2D2",
			message: "message",
		};

		let _ = Event::new()
			.with_retry(Duration::from_secs(5))
			.with_json_data(data)
			.unwrap()
			.with_data("data");
	}

	// -------------------------
	// retry type

	#[test]
	#[should_panic = "'retry' cannot be set multiple times"]
	fn multiple_retries() {
		let _ = Event::new()
			.with_name("test")
			.with_retry(Duration::from_secs(5))
			.with_retry(Duration::from_secs(10));
	}

	// -------------------------
	// comment

	#[test]
	#[should_panic = "'comment' field value cannot contain any of these ['\\r', '\\n'] characters"]
	fn comment_forbidden_chars_1() {
		let _ = Event::new().with_id("42").with_comment("\rcomment");
	}

	#[test]
	#[should_panic = "'comment' field value cannot contain any of these ['\\r', '\\n'] characters"]
	fn comment_forbidden_chars_2() {
		let _ = Event::new().with_id("42").with_comment("\ncomment");
	}

	// --------------------------------------------------

	#[tokio::test]
	async fn event_stream() {
		use futures_util::stream::unfold;
		// --------------------------------------------------------------------------------
		// --------------------------------------------------------------------------------

		let data = &Data {
			name: "C-3PO",
			message: "message",
		};

		// -------------------------

		let stream = unfold(0, move |number| async move {
			if number == 0 {
				tokio::time::sleep(Duration::from_secs(1)).await;

				let event = Event::new()
					.with_name("test")
					.with_id("42")
					.with_data("data 1\ndata 2");

				return Some((event, number + 1));
			}

			if number == 1 {
				tokio::time::sleep(Duration::from_secs(1)).await;

				let event = Event::new()
					.with_id("42")
					.with_json_data(data)
					.unwrap()
					.with_comment("...")
					.with_retry(Duration::from_secs(1));

				return Some((event, number + 1));
			}

			None
		});

		let stream_response = EventStream::new(stream)
			.with_keep_alive_interval(Some(Duration::from_millis(750)))
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
