//! A minimal SplitMix64 generator for deriving seeded initial state.
//!
//! Not a general-purpose PRNG dependency: seeded scenarios need a small,
//! fixed, portable generator whose output is identical across platforms and
//! across `rand` crate versions, since golden tests assert exact `f64`
//! equality (ADR-0008's deterministic conformance anchor). SplitMix64 is
//! specified completely by its constants, so implementing it inline avoids
//! taking on an external RNG's version and feature-flag surface.
/// A SplitMix64 pseudo-random generator, seeded from a single `u64`.
#[derive(Debug, Clone, Copy)]
pub struct SplitMix64 {
    state: u64,
}

impl SplitMix64 {
    /// Constructs a generator seeded with `seed`.
    #[must_use]
    pub const fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    /// Returns the next pseudo-random `u64` and advances the generator.
    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Returns the next pseudo-random value as an `f64` in `[0.0, 1.0)`.
    pub fn next_f64(&mut self) -> f64 {
        // 53 bits of mantissa precision, the same technique used by
        // reference SplitMix64/xoshiro implementations for `[0, 1)` floats.
        let bits = self.next_u64() >> 11;
        (bits as f64) * (1.0 / ((1_u64 << 53) as f64))
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic)]
mod tests {
    use super::SplitMix64;

    #[test]
    fn same_seed_produces_same_sequence() {
        let mut a = SplitMix64::new(42);
        let mut b = SplitMix64::new(42);
        for _ in 0..8 {
            assert_eq!(a.next_u64(), b.next_u64());
        }
    }

    #[test]
    fn different_seeds_diverge() {
        let mut a = SplitMix64::new(1);
        let mut b = SplitMix64::new(2);
        assert_ne!(a.next_u64(), b.next_u64());
    }

    #[test]
    fn next_f64_stays_in_unit_range() {
        let mut rng = SplitMix64::new(7);
        for _ in 0..64 {
            let value = rng.next_f64();
            assert!((0.0..1.0).contains(&value));
        }
    }

    #[test]
    fn known_seed_matches_fixed_golden_value() {
        // Pins the constants above against a value computed once and
        // recorded here, so an accidental edit to the algorithm is caught
        // instead of silently changing every downstream scenario and golden
        // trajectory that depends on this generator.
        let mut rng = SplitMix64::new(0);
        assert_eq!(rng.next_u64(), 0xE220_A839_7B1D_CDAF_u64);
    }
}
