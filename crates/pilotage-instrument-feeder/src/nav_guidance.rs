//! Navigation-guidance freshness, fail closed (ADR-0031).
//!
//! A guidance sample is accepted only when its COMPLETE stamp validates
//! for the navigation-solution role; the source identity is pinned at
//! first acceptance; and the epoch/sequence pair must strictly ADVANCE
//! in wrapping serial order, so a duplicate or reordered sample never
//! refreshes age and never regresses the displayed leg. Sample values
//! stay raw here — meters and radians — and reach the instrument's dot
//! scale only through [`crate::nav_display`]. Age is reported rather
//! than judged: the panel's freshness policy owns when guidance stops
//! showing, so there is no second staleness rule to diverge from it.

use pilotage_instrument_state::IdentStr;

use crate::stamp::{ROLE_NAVIGATION_SOLUTION, RawStamp, serial_is_newer, stamp_fault_for_role};

/// One decoded guidance sample in wire-canonical units.
///
/// The lane is f32-canonical: values cross this boundary as f32 —
/// matching the state frame — so a shell that computed guidance in f64
/// publishes the f32-rounded values from here on.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Guidance {
    /// Active (TO) waypoint ident.
    pub to_ident: IdentStr,
    /// Previous (FROM) waypoint ident.
    pub from_ident: IdentStr,
    /// Desired course, radians from true north.
    pub course_rad: f32,
    /// Cross-track deviation, meters, positive right of course; NaN is
    /// the schema's "not tracking" encoding and stays legal.
    pub lateral_deviation_m: f32,
    /// Vertical deviation, meters, positive above profile; NaN when
    /// unconstrained.
    pub vertical_deviation_m: f32,
    /// Distance to the active waypoint, meters.
    pub distance_to_waypoint_m: f32,
    /// Active leg index.
    pub leg_index: u32,
    /// Plan waypoint count.
    pub waypoint_count: u32,
    /// Solution quality code.
    pub solution_quality: u32,
}

impl Guidance {
    /// Guidance whose numbers are not numbers at all is unusable whole:
    /// a non-finite course or distance has no display. NaN deviations
    /// stay legal.
    pub fn is_well_formed(&self) -> bool {
        self.course_rad.is_finite() && self.distance_to_waypoint_m.is_finite()
    }
}

/// Why the tracker refused a sample.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavReject {
    /// The stamp failed its role validation.
    InvalidStamp,
    /// A different source id or incarnation than the pinned stream.
    WrongSource,
    /// A duplicate or serially older epoch/sequence pair.
    Duplicate,
    /// The sample's own values are uninterpretable.
    MalformedGuidance,
}

/// Wrap-counted acceptance and refusal counters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct NavCounters {
    /// Samples accepted and published.
    pub accepted: u32,
    /// Samples refused for a stamp that failed role validation.
    pub invalid_stamps: u32,
    /// Samples refused for a source other than the pinned stream.
    pub wrong_source: u32,
    /// Samples refused as duplicates or serially older pairs.
    pub duplicates: u32,
    /// Samples refused for uninterpretable guidance values.
    pub malformed_guidance: u32,
}

/// The newest accepted sample with its caller-clock age.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NavSnapshot {
    /// The accepted guidance sample.
    pub guidance: Guidance,
    /// Milliseconds since acceptance, on the caller's clock.
    pub age_ms: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Last {
    guidance: Guidance,
    source_id: u64,
    incarnation: [u8; 16],
    epoch: u32,
    sequence: u32,
    first_seen_ms: f64,
}

/// Tracks the single pinned navigation-guidance stream.
#[derive(Debug, Default)]
pub struct NavGuidanceTracker {
    last: Option<Last>,
    last_reject: Option<NavReject>,
    counters: NavCounters,
}

impl NavGuidanceTracker {
    /// A tracker that has seen nothing.
    pub fn new() -> Self {
        Self::default()
    }

    /// Feeds one decoded guidance sample (or `None`) and returns the
    /// current snapshot. Only a NEW sample restarts the age clock.
    pub fn observe(
        &mut self,
        sample: Option<&(RawStamp, Guidance)>,
        now_ms: f64,
    ) -> Option<NavSnapshot> {
        if let Some((stamp, guidance)) = sample
            && self.accepts(stamp, guidance)
        {
            self.last = Some(Last {
                guidance: *guidance,
                source_id: stamp.source_id,
                incarnation: stamp.incarnation,
                epoch: stamp.epoch,
                sequence: stamp.sequence,
                first_seen_ms: now_ms,
            });
            self.counters.accepted = self.counters.accepted.wrapping_add(1);
        }
        self.snapshot(now_ms)
    }

    /// Whether a sample is a valid, strictly-new observation from the
    /// pinned source. Every rejection is fail-closed: the previous
    /// snapshot (and its age) stands, so guidance goes stale rather
    /// than jumping to a leg that failed its provenance check.
    pub fn accepts(&mut self, stamp: &RawStamp, guidance: &Guidance) -> bool {
        if stamp_fault_for_role(stamp, ROLE_NAVIGATION_SOLUTION).is_some() {
            return self.reject(NavReject::InvalidStamp);
        }
        if !guidance.is_well_formed() {
            return self.reject(NavReject::MalformedGuidance);
        }
        let Some(last) = &self.last else {
            return true;
        };
        // Identity is pinned for the session: a different source id or
        // incarnation is not this navigation component's stream.
        if stamp.source_id != last.source_id || stamp.incarnation != last.incarnation {
            return self.reject(NavReject::WrongSource);
        }
        if stamp.epoch == last.epoch {
            return serial_is_newer(stamp.sequence, last.sequence)
                || self.reject(NavReject::Duplicate);
        }
        // A newer epoch (navigation restart) restarts the numbering; an
        // older epoch is a replay.
        serial_is_newer(stamp.epoch, last.epoch) || self.reject(NavReject::Duplicate)
    }

    /// The display view: `None` before any accepted sample.
    pub fn snapshot(&self, now_ms: f64) -> Option<NavSnapshot> {
        self.last.as_ref().map(|last| NavSnapshot {
            guidance: last.guidance,
            age_ms: now_ms - last.first_seen_ms,
        })
    }

    /// Acceptance/refusal counters plus the last refusal reason.
    pub fn diagnostics(&self) -> (NavCounters, Option<NavReject>) {
        (self.counters, self.last_reject)
    }

    fn reject(&mut self, reason: NavReject) -> bool {
        self.last_reject = Some(reason);
        let counter = match reason {
            NavReject::InvalidStamp => &mut self.counters.invalid_stamps,
            NavReject::WrongSource => &mut self.counters.wrong_source,
            NavReject::Duplicate => &mut self.counters.duplicates,
            NavReject::MalformedGuidance => &mut self.counters.malformed_guidance,
        };
        *counter = counter.wrapping_add(1);
        false
    }
}

#[cfg(test)]
mod tests;
