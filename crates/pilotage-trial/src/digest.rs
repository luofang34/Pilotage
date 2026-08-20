//! A fixed SHA-256 digest value.

use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer, de};

const DIGEST_BYTES: usize = 32;
const DIGEST_HEX_BYTES: usize = DIGEST_BYTES * 2;

/// A SHA-256 digest.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Digest([u8; DIGEST_BYTES]);

impl Digest {
    /// Creates a digest from its bytes.
    #[must_use]
    pub const fn from_bytes(bytes: [u8; DIGEST_BYTES]) -> Self {
        Self(bytes)
    }

    /// Gets the digest bytes.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; DIGEST_BYTES] {
        &self.0
    }

    /// Reports if all digest bytes are zero.
    #[must_use]
    pub fn is_zero(self) -> bool {
        self.0.iter().all(|byte| *byte == 0)
    }
}

impl fmt::Display for Digest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            write!(formatter, "{byte:02x}")?;
        }
        Ok(())
    }
}

impl Serialize for Digest {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Digest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        decode_hex(&value).map_err(de::Error::custom)
    }
}

fn decode_hex(value: &str) -> Result<Digest, &'static str> {
    if value.len() != DIGEST_HEX_BYTES {
        return Err("a digest must contain 64 hexadecimal characters");
    }
    let mut output = [0_u8; DIGEST_BYTES];
    for (index, pair) in value.as_bytes().as_chunks::<2>().0.iter().enumerate() {
        let high = decode_nibble(pair[0])?;
        let low = decode_nibble(pair[1])?;
        output[index] = (high << 4) | low;
    }
    Ok(Digest(output))
}

fn decode_nibble(value: u8) -> Result<u8, &'static str> {
    match value {
        b'0'..=b'9' => Ok(value - b'0'),
        b'a'..=b'f' => Ok(value - b'a' + 10),
        b'A'..=b'F' => Ok(value - b'A' + 10),
        _ => Err("a digest contains a non-hexadecimal character"),
    }
}
