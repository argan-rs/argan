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
		Self::try_new(stream).unwrap()
	}

	pub fn try_new(stream: S) -> Result<Self, EventStreamError> {
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
					interval.reset_with_duration(duration);

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

pub struct Event {
	buffer: BytesMut,
	flags: EventFlags,
}

impl Event {
	#[inline]
	pub fn with_name<T: AsRef<str>>(mut self, name: T) -> Self {
		self.try_set_name(name).unwrap();

		self
	}

	pub fn try_set_name<T: AsRef<str>>(&mut self, name: T) -> Result<(), EventStreamError> {
		if self.flags.has(EventFlags::NAME) {
			return Err(EventStreamError::CannotBeSetMultipleTimes("name"));
		}

		self.try_add_field("event", name.as_ref(), &['\r', '\n']);
		self.flags.add(EventFlags::NAME);

		Ok(())
	}

	#[inline]
	pub fn with_data<T: AsRef<str>>(mut self, data: T) -> Self {
		self.try_add_data(data).unwrap();

		self
	}

	pub fn try_add_data<T: AsRef<str>>(&mut self, data: T) -> Result<(), EventStreamError> {
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
					self.try_add_field("data", value, &[]);

					next_segment_index = i + 1;
				}
				'\n' => {
					if previous_char == '\r' {
						previous_char = ch;

						continue;
					}

					let value = &value_str[next_segment_index..i];
					self.try_add_field("data", value, &[]);

					next_segment_index = i + 1;
				}
				_ => {}
			}

			previous_char = ch;
		}

		self.flags.add(EventFlags::DATA);

		Ok(())
	}

	#[inline]
	pub fn with_json_data<T: Serialize>(mut self, json_data: T) -> Self {
		self.try_set_json_data(json_data).unwrap();

		self
	}

	pub fn try_set_json_data<T: Serialize>(&mut self, json_data: T) -> Result<(), EventStreamError> {
		if self
			.flags
			.has_any(EventFlags::DATA | EventFlags::SERIALIZED_DATA)
		{
			return Err(EventStreamError::CannotCoexistWithExistingData);
		}

		let json = serde_json::to_string(&json_data)?;
		self
			.try_add_data(&json)
			.expect("shouldn't be called when DATA or SERIALIZED_DATA flags exist");

		self.flags.add(EventFlags::SERIALIZED_DATA);

		Ok(())
	}

	#[inline]
	pub fn with_id<T: AsRef<str>>(mut self, id: T) -> Self {
		self.try_set_id(id).unwrap();

		self
	}

	pub fn try_set_id<T: AsRef<str>>(&mut self, id: T) -> Result<(), EventStreamError> {
		if self.flags.has(EventFlags::ID) {
			return Err(EventStreamError::CannotBeSetMultipleTimes("id"));
		}

		self.try_add_field("id", id.as_ref(), &['\r', '\n', '\0'])?;
		self.flags.add(EventFlags::ID);

		Ok(())
	}

	#[inline]
	pub fn with_retry(mut self, duration: Duration) -> Self {
		self.try_set_retry(duration).unwrap();

		self
	}

	pub fn try_set_retry(&mut self, duration: Duration) -> Result<(), EventStreamError> {
		if self.flags.has(EventFlags::RETRY) {
			return Err(EventStreamError::CannotBeSetMultipleTimes("retry"));
		}

		self.try_add_field("retry", &duration.as_millis().to_string(), &[])?;
		self.flags.add(EventFlags::RETRY);

		Ok(())
	}

	#[inline]
	pub fn with_comment<T: AsRef<str>>(mut self, comment: T) -> Self {
		self.try_add_comment(comment).unwrap();

		self
	}

	#[inline]
	pub fn try_add_comment<T: AsRef<str>>(&mut self, comment: T) -> Result<(), EventStreamError> {
		self.try_add_field("", comment.as_ref(), &['\r', '\n'])?;

		Ok(())
	}

	#[inline]
	fn try_add_field(
		&mut self,
		field: &'static str,
		value: &str,
		forbiddin_chars: &[char],
	) -> Result<(), EventStreamError> {
		if !forbiddin_chars.is_empty() && value.contains(forbiddin_chars) {
			return Err(EventStreamError::ForbiddenChars(
				field,
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
			.try_add_comment("")
			.expect("empty comment must be valid");

		keep_alive_event.into_bytes()
	}
}

impl Default for Event {
	#[inline(always)]
	fn default() -> Self {
		Self {
			buffer: BytesMut::new(),
			flags: EventFlags::new(),
		}
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
	#[error("'{0}' field value cannot contain any of these [{:?}] characters", .1.as_slice())]
	ForbiddenChars(&'static str, Vec<char>),
}

impl IntoResponse for EventStreamError {
	fn into_response(self) -> Response {
		StatusCode::INTERNAL_SERVER_ERROR.into_response()
	}
}

// --------------------------------------------------------------------------------
