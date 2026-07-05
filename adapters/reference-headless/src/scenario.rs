//! Seeded scenario construction: derives a reproducible initial `SkiffState`
//! from a `u64` seed (ADR-0013's deterministic-reset requirement).

use crate::skiff::SkiffState;
use crate::splitmix64::SplitMix64;

/// Half-width, in units, of the square region initial positions are drawn
/// from.
const SPAWN_HALF_EXTENT: f64 = 5.0;

/// Derives a reproducible initial `SkiffState` from `seed`.
///
/// The same seed always produces the same state, and different seeds
/// diverge; scenario placement uses only this generator, never any other
/// source of randomness, so a recorded seed fully determines a scenario's
/// starting conditions.
#[must_use]
pub fn initial_state_from_seed(seed: u64) -> SkiffState {
    let mut rng = SplitMix64::new(seed);
    let x = (rng.next_f64() * 2.0 - 1.0) * SPAWN_HALF_EXTENT;
    let y = (rng.next_f64() * 2.0 - 1.0) * SPAWN_HALF_EXTENT;
    let heading = rng.next_f64() * std::f64::consts::TAU;
    SkiffState {
        pos: [x, y],
        heading,
        speed: 0.0,
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic)]
mod tests {
    use super::initial_state_from_seed;

    #[test]
    fn same_seed_yields_identical_initial_state() {
        let a = initial_state_from_seed(1234);
        let b = initial_state_from_seed(1234);
        assert_eq!(a, b);
    }

    #[test]
    fn different_seeds_yield_different_initial_state() {
        let a = initial_state_from_seed(1);
        let b = initial_state_from_seed(2);
        assert_ne!(a, b);
    }

    #[test]
    fn initial_speed_is_always_zero() {
        let state = initial_state_from_seed(99);
        assert_eq!(state.speed, 0.0);
    }
}
