//! Configuration options of the FileStream.

// ----------

use super::*;

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

// --------------------------------------------------
// ContentCoding

/// A configuration option to choose dynamic content encoding.
#[non_exhaustive]
#[derive(Debug, PartialEq)]
pub enum ContentCoding {
	Gzip(u32),    // Gzip cooding with level.
	Deflate(u32), // Deflate coding with level.
	Brotli(u32),  // Brotli coding with level.
}

// --------------------------------------------------
// ConfigFlags

bit_flags! {
	#[derive(Default)]
	pub(super) ConfigFlags: u8 {
		pub(super) NONE = 0b00;
		pub(super) ATTACHMENT = 0b0001;
		pub(super) PARTIAL_CONTENT_SUPPORT = 0b0010;
	}
}

// --------------------------------------------------
// FileStreamProperty

option! {
	pub(super) FileStreamProperty {
		Attachment(bool),
		PartialContentSupport(bool),
		ContentEncoding(HeaderValue),
		ContentType(HeaderValue),
		Boundary(Box<str>),
		FileName(Box<str>),
	}
}

// -------------------------

/// Configures the `FileStream` to stream the file as an attachment.
///
/// Defaults to `false`.
pub struct Attachment;

impl Attachment {
	#[inline(always)]
	pub fn to(self, enabled: bool) -> FileStreamProperty {
		FileStreamProperty::Attachment(enabled)
	}
}

// -------------------------

/// Configures the FileStream to support partial content.
///
/// When FileStream is returned as a response to stream a whole file, it sets the
/// `Accept-Ranges` header to `bytes`.
///
/// When opened with ranges, defaults to `true`, otherwise to `false`.
pub struct PartialContentSupport;

impl PartialContentSupport {
	/// # Panics
	/// - if the FileStream was opened or created with encoding
	/// - if the FileStream was opened or created with ranges and `enabled` is `false`
	#[inline(always)]
	pub fn to(self, enabled: bool) -> FileStreamProperty {
		FileStreamProperty::PartialContentSupport(enabled)
	}
}

// -------------------------

/// Configures the FileStream to set the `Content-Encoding` header to the given value
/// when used as a response.
///
/// Can be used when streaming pre-encoded files.
pub struct ContentEncoding;

impl ContentEncoding {
	/// # Panics
	/// - if the value doesn't match the encoding the FileStream was opened or created with
	pub fn to(self, header_value: HeaderValue) -> FileStreamProperty {
		FileStreamProperty::ContentEncoding(header_value)
	}
}

// -------------------------

/// Configures the FileStream to set the `Content-Type` header to the provided value
/// when used as a response.
pub struct ContentType;

impl ContentType {
	pub fn to(self, header_value: HeaderValue) -> FileStreamProperty {
		FileStreamProperty::ContentType(header_value)
	}
}

// -------------------------

/// Configures the boundary of the `multipart/byteranges` stream.
pub struct Boundary;

impl Boundary {
	/// # Panics
	/// - if the FileStream was opened or created with encoding
	/// - if the boundary length exceeds 70 characters
	/// - if the boundary contains non-graphic ASCII characters
	pub fn to(self, boundary: Box<str>) -> FileStreamProperty {
		FileStreamProperty::Boundary(boundary)
	}
}

// -------------------------

/// Configures the FileStream file name.
pub struct FileName;

impl FileName {
	/// When the FileStream is used as a response, it sets the `filename` attribute of the
	/// `Content-Disposition` header to the passed `file_name`.
	pub fn to(self, file_name: Box<str>) -> FileStreamProperty {
		FileStreamProperty::FileName(file_name)
	}
}

// --------------------------------------------------------------------------------
