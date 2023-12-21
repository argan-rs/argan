use std::{
	fs::File,
	io::{self, Read, Seek, SeekFrom},
	ops::Range,
	path::Path,
	pin::Pin,
	task::{Context, Poll},
};

use bytes::{Bytes, BytesMut};
use hyper::body::{Body, Frame};
use pin_project::pin_project;

use crate::utils::BoxedError;

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

const BUFFER_SIZE: u64 = 8 * 1024; // ???

// -------------------------

// TODO: Options. IntoResponse.

#[pin_project]
pub struct FileStream {
	file: File,
	size_limit: u64,
}

impl FileStream {
	pub fn from_file<P: AsRef<Path>>(path: P) -> Result<Self, io::Error> {
		let file = File::open(path)?;
		let metadata = file.metadata()?;
		let file_size = metadata.len();

		Ok(Self {
			file,
			size_limit: file_size,
		})
	}

	pub fn from_file_slice<P: AsRef<Path>>(path: P, range: Range<u64>) -> Result<Self, io::Error> {
		let mut file = File::open(path)?;
		file.seek(SeekFrom::Start(range.start))?;

		let metadata = file.metadata()?;
		let remaining_size = metadata.len() - range.start;
		let mut size_limit = range.end - range.start;
		if remaining_size < size_limit {
			size_limit = remaining_size;
		}

		Ok(Self { file, size_limit })
	}
}

impl Body for FileStream {
	type Data = Bytes;
	type Error = BoxedError;

	fn poll_frame(
		self: Pin<&mut Self>,
		_cx: &mut Context<'_>,
	) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
		// Invariant: we should never read more than the size limit.
		if self.size_limit == 0 {
			return Poll::Ready(None);
		}

		// Buffer capacity keeps the invariant.
		let mut bytes_mut = if self.size_limit < BUFFER_SIZE {
			BytesMut::with_capacity(self.size_limit as usize)
		} else {
			BytesMut::with_capacity(8 * 1024)
		};

		let self_projection = self.project();

		// Reading should be quick, so we may not worry about pending.
		match self_projection.file.read(bytes_mut.as_mut()) {
			Ok(size) => {
				*self_projection.size_limit -= size as u64;

				Poll::Ready(Some(Ok(Frame::data(bytes_mut.freeze()))))
			}
			Err(error) => Poll::Ready(Some(Err(error.into()))),
		}
	}
}
