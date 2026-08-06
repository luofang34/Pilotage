//! Streaming SHA-256 for input produced incrementally.

use crate::compress::{H0, compress, state_bytes};

/// Incremental SHA-256: absorb bytes with [`Sha256Ctx::update`], then close
/// with [`Sha256Ctx::finalize`].
///
/// The digest equals [`crate::sha256`] over the concatenation of every
/// `update` slice; the FIPS 180-4 vectors and exhaustive split-point
/// equivalence tests pin that claim. The context is fixed-size and
/// allocation-free, so unbounded input can be hashed from a bounded
/// scratch buffer.
#[derive(Debug, Clone)]
pub struct Sha256Ctx {
    state: [u32; 8],
    /// Bytes carried between `update` calls; always fewer than 64.
    block: [u8; 64],
    block_len: usize,
    total_bytes: u64,
}

impl Sha256Ctx {
    /// A fresh context, equivalent to hashing the empty message so far.
    pub const fn new() -> Self {
        Self {
            state: H0,
            block: [0; 64],
            block_len: 0,
            total_bytes: 0,
        }
    }

    /// Absorbs `input` after everything absorbed before it.
    pub fn update(&mut self, mut input: &[u8]) {
        self.total_bytes = self.total_bytes.wrapping_add(input.len() as u64);
        if self.block_len > 0 {
            let space = 64 - self.block_len;
            let take = if input.len() < space {
                input.len()
            } else {
                space
            };
            let (head, rest) = input.split_at(take);
            self.block[self.block_len..self.block_len + take].copy_from_slice(head);
            self.block_len += take;
            input = rest;
            if self.block_len == 64 {
                self.state = compress(self.state, &self.block);
                self.block_len = 0;
            }
        }
        while let Some((chunk, rest)) = input.split_first_chunk::<64>() {
            self.state = compress(self.state, chunk);
            input = rest;
        }
        if !input.is_empty() {
            // The carried block is empty here: it either absorbed all of
            // `input` above or was completed and compressed.
            debug_assert!(self.block_len == 0);
            self.block[..input.len()].copy_from_slice(input);
            self.block_len = input.len();
        }
        debug_assert!(self.block_len < 64);
    }

    /// The digest of everything absorbed.
    #[must_use]
    pub fn finalize(mut self) -> [u8; 32] {
        let bit_len = self.total_bytes.wrapping_mul(8);
        self.block[self.block_len] = 0x80;
        let mut i = self.block_len + 1;
        // The carried block always has room for the terminator; when the
        // 8-byte length field does not fit as well, close this block and
        // pad a second.
        if i > 56 {
            while i < 64 {
                self.block[i] = 0;
                i += 1;
            }
            self.state = compress(self.state, &self.block);
            i = 0;
        }
        while i < 56 {
            self.block[i] = 0;
            i += 1;
        }
        self.block[56..].copy_from_slice(&bit_len.to_be_bytes());
        self.state = compress(self.state, &self.block);
        state_bytes(self.state)
    }
}

impl Default for Sha256Ctx {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests;
