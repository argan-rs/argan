use std::{
	pin::Pin,
	task::{Context, Poll},
	time::Duration,
};

use bytes::{BufMut, Bytes, BytesMut};
use futures_util::{Future, Stream};
use http::{
	header::{CACHE_CONTROL, CONTENT_TYPE},
	HeaderValue,
};
use http_body_util::BodyExt;
use pin_project::pin_project;
use serde::Serialize;

use crate::{
	body::{Body, Frame, HttpBody},
	common::{BoxedError, Interval},
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
		Self {
			inner: stream,
			keep_alive_interval: Some(Interval::new(Duration::from_secs(15))),
		}
	}

	pub fn set_keep_alive_duration(&mut self, duration: Duration) {
		self.keep_alive_interval = Some(Interval::new(duration));
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
						interval.restart();
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

pub struct Event {
	buffer: BytesMut,
	flags: EventFlags,
}

impl Event {
	#[inline]
	pub fn set_name<T>(mut self, value: T) -> Event
	where
		T: AsRef<str>,
	{
		if self.flags.has(EventFlags::NAME) {
			panic!("Event name can be set only once")
		}

		self.add_field("event", value.as_ref(), &['\r', '\n']);
		self.flags.add(EventFlags::NAME);

		self
	}

	pub fn add_data<T>(mut self, value: T) -> Event
	where
		T: AsRef<str>,
	{
		if self.flags.has(EventFlags::SERIALIZED_DATA) {
			panic!("Event with a serialized data cannot have another 'data' field")
		}

		let value_str = value.as_ref();
		let mut value_chars = value_str.char_indices();

		let mut previous_char = ' ';
		let mut next_segment_index = 0;

		while let Some((i, ch)) = value_chars.next() {
			match ch {
				'\r' => {
					let value = &value_str[next_segment_index..i];
					self.add_field("data", value, &[]);

					next_segment_index = i + 1;
				}
				'\n' => {
					if previous_char == '\r' {
						previous_char = ch;

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

		self.flags.add(EventFlags::DATA);

		self
	}

	#[inline]
	pub fn set_json_data<T>(self, value: T) -> Result<Event, serde_json::Error>
	// Error ?
	where
		T: Serialize,
	{
		if self
			.flags
			.has_any(EventFlags::DATA | EventFlags::SERIALIZED_DATA)
		{
			panic!("json data cannot be added to the Event with data")
		}

		let json = serde_json::to_string(&value)?;

		Ok(self.add_data(&json))
	}

	#[inline]
	pub fn set_id<T>(mut self, value: T) -> Event
	where
		T: AsRef<str>,
	{
		if self.flags.has(EventFlags::ID) {
			panic!("Event id can be set only once")
		}

		self.add_field("id", value.as_ref(), &['\r', '\n', '\0']);
		self.flags.add(EventFlags::ID);

		self
	}

	#[inline]
	pub fn set_retry(mut self, duration: Duration) -> Event {
		if self.flags.has(EventFlags::RETRY) {
			panic!("Event 'retry' field can be set only once")
		}

		self.add_field("retry", &duration.as_millis().to_string(), &[]);
		self.flags.add(EventFlags::RETRY);

		self
	}

	#[inline]
	pub fn add_comment<T>(mut self, value: T) -> Event
	where
		T: AsRef<str>,
	{
		self.add_field("", value.as_ref(), &['\r', '\n']);

		self
	}

	#[inline]
	fn add_field(&mut self, field: &str, value: &str, forbiddin_chars: &[char]) {
		if !forbiddin_chars.is_empty() && value.contains(forbiddin_chars) {
			panic!(
				"Event '{}' field value cannot contain any of these characters: {:?}",
				field, forbiddin_chars,
			)
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
		Event::default().add_comment("").into_bytes()
	}
}

impl Default for Event {
	fn default() -> Self {
		Self {
			buffer: BytesMut::new(),
			flags: EventFlags::new(),
		}
	}
}

// ----------

bit_flags! {
	EventFlags: u8 {
		NAME = 0b_0000_0001;
		DATA = 0b_0000_0010;
		SERIALIZED_DATA = 0b_0000_0100;
		ID = 0b_0000_1000;
		RETRY = 0b_0001_0000;
	}
}

// --------------------------------------------------------------------------------
