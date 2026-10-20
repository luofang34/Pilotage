//! Shared secret used only at the session transport admission boundary.

use subtle::ConstantTimeEq;

/// Application-close code for a rejected session admission.
pub const ADMISSION_REJECTED_CLOSE_CODE: u32 = 0x5049_0001;

/// A 32-byte session admission secret. Debug output never exposes its bytes.
#[derive(Clone, PartialEq, Eq)]
pub struct JoinToken([u8; 32]);

/// A join token does not have the required hexadecimal representation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum JoinTokenError {
    /// The input must contain exactly 64 ASCII hexadecimal digits.
    #[error("join token must contain exactly 64 hexadecimal digits")]
    InvalidLength,
    /// At least one input byte is not an ASCII hexadecimal digit.
    #[error("join token contains a non-hexadecimal digit")]
    InvalidHex,
}

impl JoinToken {
    /// Decode exactly 64 ASCII hexadecimal digits, without whitespace.
    ///
    /// # Errors
    ///
    /// Returns [`JoinTokenError`] for an invalid length or digit.
    pub fn from_hex(value: &str) -> Result<Self, JoinTokenError> {
        if value.len() != 64 {
            return Err(JoinTokenError::InvalidLength);
        }
        let mut bytes = [0; 32];
        for (output, [high, low]) in bytes.iter_mut().zip(value.as_bytes().as_chunks::<2>().0) {
            *output = digit(*high)? * 16 + digit(*low)?;
        }
        Ok(Self(bytes))
    }

    /// Bytes carried by the existing ClientHello join-token field.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Compare a candidate without data-dependent timing for equal-length secrets.
    #[must_use]
    pub fn matches(&self, candidate: &[u8]) -> bool {
        bool::from(self.0.as_slice().ct_eq(candidate))
    }
}

impl core::fmt::Debug for JoinToken {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("JoinToken([redacted])")
    }
}

fn digit(value: u8) -> Result<u8, JoinTokenError> {
    match value {
        b'0'..=b'9' => Ok(value - b'0'),
        b'a'..=b'f' => Ok(value - b'a' + 10),
        b'A'..=b'F' => Ok(value - b'A' + 10),
        _ => Err(JoinTokenError::InvalidHex),
    }
}

#[cfg(test)]
mod tests;
