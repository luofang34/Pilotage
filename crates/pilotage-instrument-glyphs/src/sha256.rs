//! A small, const-evaluable SHA-256 over byte slices (FIPS 180-4).
//!
//! Implemented here rather than pulled from a dependency so the content
//! hash is computed with no third-party code and, being a `const fn`, is
//! evaluated at compile time to pin the recorded hash without a build
//! script or a checked-in magic literal. Correctness is anchored by the
//! published test vectors in the accompanying tests.

/// Round constants: the first 32 bits of the fractional parts of the cube
/// roots of the first 64 primes.
const K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

/// Initial hash state: the first 32 bits of the fractional parts of the
/// square roots of the first 8 primes.
const H0: [u32; 8] = [
    0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
];

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
        let mut w = [0u32; 64];
        let mut t = 0;
        while t < 16 {
            let mut word = 0u32;
            let mut b = 0;
            while b < 4 {
                let byte = padded_byte(input, block + t * 4 + b, padded_len, bit_len);
                word = (word << 8) | (byte as u32);
                b += 1;
            }
            w[t] = word;
            t += 1;
        }
        let mut i = 16;
        while i < 64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
            i += 1;
        }

        let mut a = h[0];
        let mut b = h[1];
        let mut c = h[2];
        let mut d = h[3];
        let mut e = h[4];
        let mut f = h[5];
        let mut g = h[6];
        let mut hh = h[7];
        let mut j = 0;
        while j < 64 {
            let big_s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let t1 = hh
                .wrapping_add(big_s1)
                .wrapping_add(ch)
                .wrapping_add(K[j])
                .wrapping_add(w[j]);
            let big_s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = big_s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
            j += 1;
        }

        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
        h[5] = h[5].wrapping_add(f);
        h[6] = h[6].wrapping_add(g);
        h[7] = h[7].wrapping_add(hh);
        block += 64;
    }

    let mut out = [0u8; 32];
    let mut i = 0;
    while i < 8 {
        let be = h[i].to_be_bytes();
        out[i * 4] = be[0];
        out[i * 4 + 1] = be[1];
        out[i * 4 + 2] = be[2];
        out[i * 4 + 3] = be[3];
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests;
