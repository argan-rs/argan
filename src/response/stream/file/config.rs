use crate::common::IntoArray;

use super::*;

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

bit_flags! {
	#[derive(Default)]
	pub(super) ConfigFlags: u8 {
		pub(super) NONE = 0b00;
		pub(super) ATTACHMENT = 0b0001;
		pub(super) RANGE_SUPPORT = 0b0010;
	}
}

// ----------

pub struct FileStreamConfigOption(pub(super) FileStreamConfigOptionValue);

pub(super) enum FileStreamConfigOptionValue {
	Attachment,
	SupportPartialContent,
	ContentEncoding(HeaderValue),
	ContentType(HeaderValue),
	Boundary(Box<str>),
	FileName(Box<str>),
}

impl IntoArray<FileStreamConfigOption, 1> for FileStreamConfigOption {
	fn into_array(self) -> [FileStreamConfigOption; 1] {
		[self]
	}
}

// ----------

pub fn as_attachment() -> FileStreamConfigOption {
	FileStreamConfigOption(FileStreamConfigOptionValue::Attachment)
}

pub fn support_partial_content() -> FileStreamConfigOption {
	FileStreamConfigOption(FileStreamConfigOptionValue::SupportPartialContent)
}

pub fn content_encoding(header_value: HeaderValue) -> FileStreamConfigOption {
	FileStreamConfigOption(FileStreamConfigOptionValue::ContentEncoding(header_value))
}

pub fn content_type(header_value: HeaderValue) -> FileStreamConfigOption {
	FileStreamConfigOption(FileStreamConfigOptionValue::ContentType(header_value))
}

pub fn boundary(boundary: Box<str>) -> FileStreamConfigOption {
	FileStreamConfigOption(FileStreamConfigOptionValue::Boundary(boundary))
}

pub fn file_name<F: AsRef<str>>(file_name: F) -> FileStreamConfigOption {
	let file_name = file_name.as_ref();

	let mut file_name_string = String::new();
	file_name_string.push_str("; filename");

	if file_name
		.as_bytes()
		.iter()
		.any(|ch| !ch.is_ascii_alphanumeric())
	{
		file_name_string.push_str("*=utf-8''");
		file_name_string.push_str(&percent_encode(file_name.as_bytes(), NON_ALPHANUMERIC).to_string());
	} else {
		file_name_string.push_str("=\"");
		file_name_string.push_str(&file_name);
		file_name_string.push('"');
	}

	FileStreamConfigOption(FileStreamConfigOptionValue::FileName(
		file_name_string.into(),
	))
}

// -------------------------

#[non_exhaustive]
#[derive(Debug, PartialEq)]
pub enum ContentCoding {
	Gzip(u32), // Contains level.
}
