//! A bounds-checked little-endian reader over one payload.

use super::PayloadError;

/// Reads fields in order and names the message in every truncation error.
pub(super) struct Cursor<'a> {
    message: &'static str,
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Cursor<'a> {
    pub(super) fn new(message: &'static str, bytes: &'a [u8]) -> Self {
        Self {
            message,
            bytes,
            at: 0,
        }
    }

    pub(super) fn take(&mut self, len: usize) -> Result<&'a [u8], PayloadError> {
        let end = self.at.saturating_add(len);
        let slice = self
            .bytes
            .get(self.at..end)
            .ok_or(PayloadError::Truncated {
                message: self.message,
                len: self.bytes.len(),
                needed: end,
            })?;
        self.at = end;
        Ok(slice)
    }

    pub(super) fn remaining(&self) -> usize {
        self.bytes.len().saturating_sub(self.at)
    }

    pub(super) fn u8(&mut self) -> Result<u8, PayloadError> {
        Ok(self.array::<1>()?[0])
    }

    pub(super) fn u16(&mut self) -> Result<u16, PayloadError> {
        Ok(u16::from_le_bytes(self.array()?))
    }

    pub(super) fn i16(&mut self) -> Result<i16, PayloadError> {
        Ok(i16::from_le_bytes(self.array()?))
    }

    pub(super) fn u32(&mut self) -> Result<u32, PayloadError> {
        Ok(u32::from_le_bytes(self.array()?))
    }

    pub(super) fn i32(&mut self) -> Result<i32, PayloadError> {
        Ok(i32::from_le_bytes(self.array()?))
    }

    /// The next field when the payload carries it; `None` when the payload
    /// ends first (a field a newer API version appended).
    pub(super) fn optional<T>(
        &mut self,
        read: impl FnOnce(&mut Self) -> Result<T, PayloadError>,
    ) -> Option<T> {
        read(self).ok()
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], PayloadError> {
        let mut out = [0; N];
        out.copy_from_slice(self.take(N)?);
        Ok(out)
    }
}
