// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

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
			$type: crate::utils::Uint,
		{
			#[inline(always)]
			$flags_vis fn new() -> Self {
				Self(0)
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
			$type: crate::utils::Uint,
		{
			type Output = $flags;

			fn bitor(self, rhs: Self) -> Self::Output {
				Self(self.0 | rhs.0)
			}
		}
	)
}

// --------------------------------------------------------------------------------
