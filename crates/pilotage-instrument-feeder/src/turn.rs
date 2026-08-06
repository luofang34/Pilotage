//! Measurement-coherent turn-rate derivation (DYN-01).
//!
//! Heading rate is differenced ONLY between two distinct accepted
//! heading measurements of one stream — identified by the AV-01 stamp
//! (source id + incarnation + epoch, ordered by sequence) — over the
//! MEASUREMENT acquisition clock, never render or receipt time.
//! Repeated renders of one sample re-declare the cached rate (the
//! measurement's own age carries staleness); duplicates and reordered
//! samples never advance the state; a stream discontinuity resets it,
//! so no difference can ever straddle two sessions, sources, or epochs.

use core::f64::consts::PI;

use crate::stamp::{RawStamp, serial_is_newer};

/// Closer than this between differenced samples is too noisy to
/// differentiate; no sample is declared.
pub const MIN_TURN_DT_MS: f64 = 5.0;
/// Farther than this is stale for a rate; no sample is declared.
pub const MAX_TURN_DT_MS: f64 = 500.0;

/// Heading-rate basis code in the dynamics group vocabulary.
const TURN_BASIS_HEADING_RATE: u8 = 0;

/// A dynamics declaration derived from two heading measurements.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TurnDeclaration {
    /// What the rate measures (heading rate).
    pub turn_basis: u8,
    /// Turn rate, radians/second, positive right. Derived at full
    /// precision; consumers narrow at the wire, not here.
    pub turn_rps: f64,
    /// The current measurement's age, milliseconds.
    pub age_ms: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Previous {
    heading_rad: f64,
    stamp: RawStamp,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Cached {
    turn_basis: u8,
    turn_rps: f64,
}

/// Derives heading-rate dynamics declarations from per-measurement
/// heading samples. One instance per session presentation.
#[derive(Debug, Default)]
pub struct TurnDerivation {
    prev: Option<Previous>,
    cached: Option<Cached>,
}

impl TurnDerivation {
    /// A derivation that has seen nothing.
    pub fn new() -> Self {
        Self::default()
    }

    /// Clears all state; the next sample can never difference against
    /// anything observed before the reset.
    pub fn reset(&mut self) {
        self.prev = None;
        self.cached = None;
    }

    /// Consumes the current declared heading (radians) with its
    /// measurement stamp; returns a dynamics declaration or `None`
    /// when no rate can honestly be derived.
    pub fn update(
        &mut self,
        heading_rad: f64,
        age_ms: f64,
        stamp: Option<&RawStamp>,
    ) -> Option<TurnDeclaration> {
        let Some(stamp) = stamp else {
            self.reset();
            return None;
        };
        if !heading_rad.is_finite() {
            self.reset();
            return None;
        }
        let Some(prev) = self.prev else {
            self.seed(heading_rad, *stamp);
            return None;
        };
        if !prev.stamp.same_stream(stamp) {
            self.reset();
            self.seed(heading_rad, *stamp);
            return None;
        }
        if stamp.sequence == prev.stamp.sequence {
            return self.declare(age_ms);
        }
        if !serial_is_newer(stamp.sequence, prev.stamp.sequence) {
            // A serially older sample is ignored ENTIRELY: it neither
            // advances state nor produces a declaration — its age must
            // never refresh the freshness of a rate it did not
            // contribute to.
            return None;
        }
        let dt_ms = (stamp
            .acquired_at_ns
            .saturating_sub(prev.stamp.acquired_at_ns)) as f64
            / 1.0e6;
        let prev_heading = prev.heading_rad;
        self.seed(heading_rad, *stamp);
        if !(MIN_TURN_DT_MS..=MAX_TURN_DT_MS).contains(&dt_ms) {
            self.cached = None;
            return None;
        }
        // Circular difference into (-pi, pi] so 359°→1° is +2°, never
        // −358°.
        let mut delta = (heading_rad - prev_heading) % (2.0 * PI);
        if delta > PI {
            delta -= 2.0 * PI;
        }
        if delta <= -PI {
            delta += 2.0 * PI;
        }
        self.cached = Some(Cached {
            turn_basis: TURN_BASIS_HEADING_RATE,
            turn_rps: delta / (dt_ms / 1000.0),
        });
        self.declare(age_ms)
    }

    fn seed(&mut self, heading_rad: f64, stamp: RawStamp) {
        self.prev = Some(Previous { heading_rad, stamp });
    }

    fn declare(&self, age_ms: f64) -> Option<TurnDeclaration> {
        self.cached.map(|cached| TurnDeclaration {
            turn_basis: cached.turn_basis,
            turn_rps: cached.turn_rps,
            age_ms,
        })
    }
}

#[cfg(test)]
mod tests;
