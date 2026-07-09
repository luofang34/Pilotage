//! Fixed-capacity string building over `core::fmt`, for no-alloc labels.

use core::fmt::{self, Write};

/// A stack string of at most `N` bytes implementing [`core::fmt::Write`].
///
/// Overflow returns a `fmt::Error` from `write!` instead of truncating
/// silently; callers size `N` generously for the label they format.
pub struct FixedStr<const N: usize> {
    buf: [u8; N],
    len: usize,
}

impl<const N: usize> FixedStr<N> {
    /// An empty string.
    pub const fn new() -> Self {
        Self {
            buf: [0; N],
            len: 0,
        }
    }

    /// The formatted text.
    pub fn as_str(&self) -> &str {
        core::str::from_utf8(self.buf.get(..self.len).unwrap_or(&[])).unwrap_or("")
    }
}

impl<const N: usize> Write for FixedStr<N> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        let end = self.len.checked_add(s.len()).ok_or(fmt::Error)?;
        let dst = self.buf.get_mut(self.len..end).ok_or(fmt::Error)?;
        dst.copy_from_slice(s.as_bytes());
        self.len = end;
        Ok(())
    }
}

/// Formats into a [`FixedStr`], returning an empty string on overflow —
/// a wrong-but-safe label beats a panic in drawing code.
macro_rules! fmt_label {
    ($cap:literal, $($arg:tt)*) => {{
        use core::fmt::Write as _;
        let mut s = $crate::fixed_str::FixedStr::<$cap>::new();
        core::write!(s, $($arg)*).ok();
        s
    }};
}

pub(crate) use fmt_label;
