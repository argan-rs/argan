use super::*;

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

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
// FileStreamConfigOptions

option! {
	pub(super) FileStreamConfigOption {
		Attachment,
		PartialContentSupport,
		ContentEncoding(HeaderValue),
		ContentType(HeaderValue),
		Boundary(Box<str>),
		FileName(Box<str>),
	}
}

// ----------

/// Configures the FileStream to stream the file as an attachment.
pub fn _as_attachment() -> FileStreamConfigOption {
	FileStreamConfigOption::Attachment
}

/// Configures the FileStream to support partial content.
///
/// When FileStream is returned as a response to stream a whole file, it sets the
/// `Accept-Ranges` header to `bytes`.
///
/// # Panics
/// - if the FileStream was opened or created with encoding
pub fn _to_support_partial_content() -> FileStreamConfigOption {
	FileStreamConfigOption::PartialContentSupport
}

/// Configures the FileStream to set the `Content-Encoding` header to the given value
/// when used as a response.
///
/// Can be used when streaming pre-encoded files.
///
/// # Panics
/// - if the value doesn't match the encoding the FileStream was opened or created with.
pub fn _content_encoding(header_value: HeaderValue) -> FileStreamConfigOption {
	FileStreamConfigOption::ContentEncoding(header_value)
}

/// Configures the FileStream to set the `Content-Type` header to the provided value
/// when used as a response.
pub fn _content_type(header_value: HeaderValue) -> FileStreamConfigOption {
	FileStreamConfigOption::ContentType(header_value)
}

/// Configures the boundary of the `multipart/byteranges` stream.
///
/// # Panics
/// - if the FileStream was opened or created with encoding
/// - if the boundary length exceeds 70 characters
/// - if the boundary contains non-graphic ASCII characters
pub fn _boundary(boundary: Box<str>) -> FileStreamConfigOption {
	FileStreamConfigOption::Boundary(boundary)
}

/// Configures the FileStream file name.
///
/// When the FileStream is used as a response, it sets the `filename` attribute of the
/// `Content-Disposition` header to this value.
pub fn _file_name(file_name: Box<str>) -> FileStreamConfigOption {
	FileStreamConfigOption::FileName(file_name)
}

// --------------------------------------------------------------------------------
