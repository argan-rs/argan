// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

// --------------------------------------------------------------------------------
// Config Options

macro_rules! config_option {
	(
		$(#[$metas:meta])*
		$config_name:ident {
			$($option_name:ident $(($($tokens:ty),+))?,)+
		}
	) => {
		mod config_private {
			use super::*;

			#[allow(private_interfaces)]
			$(#[$metas])*
			pub enum $config_name {
				$($option_name $(($($tokens),+))?,)+
			}

			impl IntoArray<$config_name, 1> for $config_name {
				fn into_array(self) -> [$config_name; 1] {
					[self]
				}
			}
		}

		pub(super) use config_private::$config_name;
	};
}

// --------------------------------------------------------------------------------
// Bit Flags

macro_rules! bit_flags {
	(
		$(#[$flags_meta:meta])*
		$flags_vis:vis $flags:ident: $type:ty $({
			$($name_vis:vis $name:ident = $value:literal;)*
		})?
	) => (
		$(#[$flags_meta])*
		$flags_vis struct $flags($type);

		impl $flags
		where
			$type: Copy
				+ std::ops::BitOr
				+ std::ops::BitOrAssign
				+ std::ops::BitAnd
				+ std::ops::BitAndAssign
				+ std::cmp::PartialEq
				+ std::cmp::Eq
				+ std::cmp::PartialOrd
				+ std::cmp::Ord,
		{
			#[inline(always)]
			$flags_vis fn new() -> Self
			where
				Self: Default,
			{
				Self::default()
			}

			#[inline(always)]
			$flags_vis fn add(&mut self, flags: $flags) {
				self.0 |= flags.0
			}

			#[inline(always)]
			pub(crate) fn remove(&mut self, flags: $flags) {
				self.0 &= !flags.0
			}

			#[inline(always)]
			pub(crate) fn has(&self, flags: $flags) -> bool {
				(self.0 & flags.0) == flags.0
			}

			#[inline(always)]
			pub(crate) fn has_any(&self, flags: $flags) -> bool {
				(self.0 & flags.0) > 0
			}

			#[inline(always)]
			pub(crate) fn is_empty(&self) -> bool {
				self.0 == 0
			}

			$($($name_vis const $name: $flags = $flags($value);)*)?
		}

		impl std::ops::BitOr for $flags
		where
			$type: Copy
				+ std::ops::BitOr
				+ std::ops::BitOrAssign
				+ std::ops::BitAnd
				+ std::ops::BitAndAssign
				+ std::cmp::PartialEq
				+ std::cmp::Eq
				+ std::cmp::PartialOrd
				+ std::cmp::Ord,
		{
			type Output = $flags;

			#[inline(always)]
			fn bitor(self, rhs: Self) -> Self::Output {
				Self(self.0 | rhs.0)
			}
		}
	)
}

// --------------------------------------------------------------------------------

#[rustfmt::skip]
macro_rules! call_for_tuples {
	($m:ident!) => {
		$m!(T1, TL);
		$m!(T1, (T2), TL);
		$m!(T1, (T2, T3), TL);
		$m!(T1, (T2, T3, T4), TL);
		$m!(T1, (T2, T3, T4, T5), TL);
		$m!(T1, (T2, T3, T4, T5, T6), TL);
		$m!(T1, (T2, T3, T4, T5, T6, T7), TL);
		$m!(T1, (T2, T3, T4, T5, T6, T7, T8), TL);
		$m!(T1, (T2, T3, T4, T5, T6, T7, T8, T9), TL);
		$m!(T1, (T2, T3, T4, T5, T6, T7, T8, T9, T10), TL);
		$m!(T1, (T2, T3, T4, T5, T6, T7, T8, T9, T10, T11), TL);
		$m!(T1, (T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12), TL);
		$m!(T1, (T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12, T13), TL);
		$m!(T1, (T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12, T13, T14), TL);
		$m!(T1, (T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12, T13, T14, T15), TL);
	};
}

// --------------------------------------------------------------------------------
// Data Extractor Error

macro_rules! data_extractor_error {
	(
		$(#[$enum_metas:meta])*
		$vis:vis $error_name:ident {
			$(
				$(#[$variant_metas:meta])*
				($field_name:ident $($field_contents:tt)?) $([$match_contents:tt];)? $status_code:path;
			)*
		}
	) => {
		#[non_exhaustive]
		#[allow(non_snake_case)]
		$(#[$enum_metas])*
		#[derive(crate::ImplError)]
		$vis enum $error_name {
			#[error("missing Content-Type")]
			MissingContentType,
			#[error("invalid Content-Type: {0}")]
			InvalidContentType(http::header::ToStrError),
			#[error("unsupported media type")]
			UnsupportedMediaType,
			#[error("content too large")]
			ContentTooLarge,
			#[error("buffering failure")]
			BufferingFailure,
			$(
				$(#[$variant_metas])*
				$field_name $($field_contents)?
			),*
		}

		impl From<crate::data::header::ContentTypeError> for $error_name {
			fn from(header_error: crate::data::header::ContentTypeError) -> Self {
				match header_error {
					crate::data::header::ContentTypeError::Missing => $error_name::MissingContentType,
					crate::data::header::ContentTypeError::InvalidValue(error) => {
						$error_name::InvalidContentType(error)
					}
				}
			}
		}

		impl IntoResponse for $error_name {
			fn into_response(self) -> Response {
				use $error_name::*;

				match self {
					MissingContentType | InvalidContentType(_) => {
						StatusCode::BAD_REQUEST.into_response()
					},
					UnsupportedMediaType => StatusCode::UNSUPPORTED_MEDIA_TYPE.into_response(),
					ContentTooLarge => StatusCode::PAYLOAD_TOO_LARGE.into_response(),
					BufferingFailure => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
					$(
						$field_name $($match_contents)? => $status_code.into_response()
					),*
				}
			}
		}
	};
}

// --------------------------------------------------------------------------------
