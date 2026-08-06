//! One-shot, const-evaluable SHA-256 over a complete byte slice.

use crate::compress::{H0, compress, state_bytes};

/// The byte at `idx` of the padded message: the input, the single `0x80`
/// terminator, zero padding, then the 64-bit big-endian bit length.
const fn padded_byte(input: &[u8], idx: usize, padded_len: usize, bit_len: u64) -> u8 {
    let len = input.len();
    if idx < len {
        input[idx]
    } else if idx == len {
        0x80
    } else if idx + 8 >= padded_len {
        let shift = (padded_len - 1 - idx) * 8;
        ((bit_len >> shift) & 0xff) as u8
    } else {
        0
    }
}

/// The SHA-256 digest of `input`.
#[must_use]
pub const fn sha256(input: &[u8]) -> [u8; 32] {
    let len = input.len();
    let bit_len = (len as u64).wrapping_mul(8);
    let mut padded_len = len + 1 + 8;
    let rem = padded_len % 64;
    if rem != 0 {
        padded_len += 64 - rem;
    }

    let mut h = H0;
    let mut block = 0;
    while block < padded_len {
        let mut bytes = [0u8; 64];
        let mut i = 0;
        while i < 64 {
            bytes[i] = padded_byte(input, block + i, padded_len, bit_len);
            i += 1;
        }
        h = compress(h, &bytes);
        block += 64;
    }
    state_bytes(h)
}

#[cfg(test)]
mod tests;
