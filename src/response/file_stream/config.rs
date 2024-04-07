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

pub fn _as_attachment() -> FileStreamConfigOption {
	FileStreamConfigOption::Attachment
}

pub fn _to_support_partial_content() -> FileStreamConfigOption {
	FileStreamConfigOption::PartialContentSupport
}

pub fn _content_encoding(header_value: HeaderValue) -> FileStreamConfigOption {
	FileStreamConfigOption::ContentEncoding(header_value)
}

pub fn _content_type(header_value: HeaderValue) -> FileStreamConfigOption {
	FileStreamConfigOption::ContentType(header_value)
}

pub fn _boundary(boundary: Box<str>) -> FileStreamConfigOption {
	FileStreamConfigOption::Boundary(boundary)
}

pub fn _file_name(file_name: Box<str>) -> FileStreamConfigOption {
	FileStreamConfigOption::FileName(file_name)
}

// --------------------------------------------------------------------------------
