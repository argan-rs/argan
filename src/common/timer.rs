use std::{
	future::Future,
	pin::Pin,
	sync::OnceLock,
	task::{Context, Poll},
	time::{Duration, Instant},
};

use http::StatusCode;
use hyper::rt::{Sleep, Timer as RuntimeTimer};

use crate::response::IntoResponse;

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

static TIMER: OnceLock<Timer> = OnceLock::new();

pub(crate) fn set_timer<T>(timer: T)
where
	T: RuntimeTimer + Send + Sync + 'static,
{
	TIMER.set(Timer(Box::new(timer)));
}

// --------------------------------------------------

pub(crate) struct Timer(Box<dyn hyper::rt::Timer + Send + Sync>);

impl Timer {
	#[inline(always)]
	pub(crate) fn sleep(&self, duration: Duration) -> Pin<Box<dyn Sleep>> {
		self.0.sleep(duration)
	}

	#[inline(always)]
	pub(crate) fn sleep_until(&self, deadline: Instant) -> Pin<Box<dyn Sleep>> {
		self.0.sleep_until(deadline)
	}

	#[inline(always)]
	pub(crate) fn reset(&self, sleep: &mut Pin<Box<dyn Sleep>>, new_deadline: Instant) {
		self.0.reset(sleep, new_deadline)
	}
}

// --------------------------------------------------

pub(crate) struct Interval {
	duration: Duration,
	sleep: Pin<Box<dyn Sleep>>,
}

impl Interval {
	pub(crate) fn try_new(duration: Duration) -> Result<Self, UninitializedTimer> {
		let timer = TIMER.get().ok_or(UninitializedTimer)?;
		let sleep = timer.sleep(duration);

		Ok(Self { duration, sleep })
	}

	pub(crate) fn set_duration(&mut self, new_duration: Duration) {
		self.duration = new_duration;
	}

	pub(crate) fn reset(&mut self) {
		let timer = TIMER
			.get()
			.expect("a valid instance of Interval should prove the TIMER was initialized");

		timer.reset(&mut self.sleep, Instant::now() + self.duration)
	}

	pub(crate) fn reset_with_duration(&mut self, duration: Duration) {
		let timer = TIMER
			.get()
			.expect("a valid instance of Interval should prove the TIMER was initialized");

		timer.reset(&mut self.sleep, Instant::now() + duration)
	}

	pub(crate) fn pin(&mut self) -> Pin<&mut Self> {
		Pin::new(self)
	}
}

impl Future for Interval {
	type Output = ();

	fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
		match self.sleep.as_mut().poll(cx) {
			Poll::Ready(_) => {
				self.reset();

				Poll::Ready(())
			}
			Poll::Pending => Poll::Pending,
		}
	}
}

// --------------------------------------------------

#[derive(Debug, crate::ImplError)]
#[error("uninitialized timer")]
pub struct UninitializedTimer;

impl IntoResponse for UninitializedTimer {
	#[inline(always)]
	fn into_response(self) -> crate::response::Response {
		StatusCode::INTERNAL_SERVER_ERROR.into_response()
	}
}

// --------------------------------------------------------------------------------
