//! Reorder-safe avionics ingestion for the operational-estimate lanes.
//!
//! Publication/receipt time is transport metadata: freshness advances
//! only when a source group presents a new epoch/sequence. Each accepted
//! estimator status opens an authorization regime; numeric groups are
//! judged by the regime that governed their acquisition instant, a
//! duplicate status may only tighten authorization (monotone fold), and
//! everything unpairable fails closed to no-valid-flags and unusable
//! quality.

use crate::stamp::{
    ROLE_OPERATIONAL_ESTIMATE, RawStamp, StampFault, serial_distance, serial_is_newer, skew_ns,
    stamp_fault_for_role,
};

mod types;

pub use types::{
    AttitudeGroup, AvionicsSample, Coherence, CoherenceReport, GroupSnapshot, IncarnationPolicy,
    IngressConfig, IngressCounters, IngressSnapshot, KinematicsGroup,
};

const QUALITY_UNUSABLE: u32 = 2;
const ATTITUDE_VALID_FLAGS: u32 = 0b0011;
const KINEMATICS_VALID_FLAGS: u32 = 0b1100;
const KNOWN_VALID_FLAGS: u32 = ATTITUDE_VALID_FLAGS | KINEMATICS_VALID_FLAGS;

/// Bound on remembered incarnations under
/// [`IncarnationPolicy::SimAcceptUnseen`].
pub const MAX_SEEN_INCARNATIONS: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq)]
struct Group<T> {
    stamp: RawStamp,
    data: T,
    accepted_at_ms: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct StatusRegime {
    source_id: u64,
    incarnation: [u8; 16],
    epoch: u32,
    acquired_at_ns: u64,
    clock: u8,
    valid_flags: u32,
    quality: u32,
}

impl StatusRegime {
    /// Whether this regime's status can vouch for a numeric group
    /// acquired at `numeric`: identical stream and an acquisition gap
    /// within the coherence budget (lanes interleave at their own
    /// rates; demanding the exact instant would flash the panels).
    fn pairs(&self, numeric: &RawStamp, maximum_skew_nanos: u64) -> bool {
        self.source_id == numeric.source_id
            && self.incarnation == numeric.incarnation
            && self.epoch == numeric.epoch
            && self.clock == numeric.clock
            && skew_ns(numeric.acquired_at_ns, self.acquired_at_ns) <= maximum_skew_nanos
    }
}

/// The ingress gate for one vehicle's operational-estimate lanes.
#[derive(Debug)]
pub struct AvionicsIngress {
    config: IngressConfig,
    source_id: Option<u64>,
    incarnation: Option<[u8; 16]>,
    seen: [Option<[u8; 16]>; MAX_SEEN_INCARNATIONS],
    epoch: Option<u32>,
    attitude: Option<Group<AttitudeGroup>>,
    kinematics: Option<Group<KinematicsGroup>>,
    estimator_status: Option<Group<()>>,
    regime: Option<StatusRegime>,
    previous_regime: Option<StatusRegime>,
    attitude_paired: bool,
    kinematics_paired: bool,
    valid_flags: u32,
    quality: u32,
    generation: u32,
    last_coherence: Coherence,
    last_reject: Option<StampFault>,
    counters: IngressCounters,
}

impl AvionicsIngress {
    /// A gate that has seen nothing beyond any pre-pinned identity.
    pub fn new(config: IngressConfig) -> Self {
        let mut seen = [None; MAX_SEEN_INCARNATIONS];
        if let Some(pinned) = config.incarnation {
            seen[0] = Some(pinned);
        }
        Self {
            config,
            source_id: config.source_id,
            incarnation: config.incarnation,
            seen,
            epoch: None,
            attitude: None,
            kinematics: None,
            estimator_status: None,
            regime: None,
            previous_regime: None,
            attitude_paired: false,
            kinematics_paired: false,
            valid_flags: 0,
            quality: QUALITY_UNUSABLE,
            generation: 0,
            last_coherence: Coherence::Insufficient,
            last_reject: None,
            counters: IngressCounters::default(),
        }
    }

    /// Ingests one publication; returns whether admitted state changed.
    pub fn ingest(&mut self, sample: &AvionicsSample, now_ms: f64) -> bool {
        if sample.vehicle_id != self.config.vehicle_id {
            self.counters.wrong_vehicle = self.counters.wrong_vehicle.wrapping_add(1);
            return false;
        }
        let accepted_status = self.accept_status(sample, now_ms);
        let accepted_attitude = self.accept_attitude(sample, now_ms);
        let accepted_kinematics = self.accept_kinematics(sample, now_ms);
        let previous_valid = self.valid_flags;
        let previous_quality = self.quality;
        self.update_authorization(sample, accepted_attitude, accepted_kinematics);
        let changed = accepted_attitude
            || accepted_kinematics
            || accepted_status
            || self.valid_flags != previous_valid
            || self.quality != previous_quality;
        if changed {
            self.generation = self.generation.wrapping_add(1);
            self.record_coherence_transition();
        }
        changed
    }

    fn accept_status(&mut self, sample: &AvionicsSample, now_ms: f64) -> bool {
        let Some(stamp) = sample.estimator_status_stamp else {
            return false;
        };
        if !self.admit_identity(&stamp) {
            return false;
        }
        // The slot is read only after identity admission: a source reset
        // above just cleared it, and ordering against a cleared stream
        // would judge the new stream by the dead one's numbering.
        let slot = self.estimator_status.map(|group| group.stamp);
        if !self.admit_ordering(&stamp, slot) {
            return false;
        }
        self.estimator_status = Some(Group {
            stamp,
            data: (),
            accepted_at_ms: now_ms,
        });
        // Each accepted status opens a new authorization regime; the
        // one it closes is retained so an interleaved numeric acquired
        // under the closed regime is judged by the estimator state that
        // governed its acquisition instant.
        self.previous_regime = self.regime;
        self.regime = Some(StatusRegime {
            source_id: stamp.source_id,
            incarnation: stamp.incarnation,
            epoch: stamp.epoch,
            acquired_at_ns: stamp.acquired_at_ns,
            clock: stamp.clock,
            valid_flags: sample.valid_flags,
            quality: sample.quality,
        });
        true
    }

    fn accept_attitude(&mut self, sample: &AvionicsSample, now_ms: f64) -> bool {
        let Some(stamp) = sample.attitude_stamp else {
            return false;
        };
        if !self.admit_identity(&stamp) {
            return false;
        }
        let slot = self.attitude.map(|group| group.stamp);
        if !self.admit_ordering(&stamp, slot) {
            return false;
        }
        self.attitude = Some(Group {
            stamp,
            data: sample.attitude,
            accepted_at_ms: now_ms,
        });
        true
    }

    fn accept_kinematics(&mut self, sample: &AvionicsSample, now_ms: f64) -> bool {
        let Some(stamp) = sample.kinematics_stamp else {
            return false;
        };
        if !self.admit_identity(&stamp) {
            return false;
        }
        let slot = self.kinematics.map(|group| group.stamp);
        if !self.admit_ordering(&stamp, slot) {
            return false;
        }
        self.kinematics = Some(Group {
            stamp,
            data: sample.kinematics,
            accepted_at_ms: now_ms,
        });
        true
    }

    /// Stamp validity and source/incarnation/epoch identity admission,
    /// shared by every group stream.
    fn admit_identity(&mut self, stamp: &RawStamp) -> bool {
        if let Some(fault) = stamp_fault_for_role(stamp, ROLE_OPERATIONAL_ESTIMATE) {
            self.counters.invalid_stamps = self.counters.invalid_stamps.wrapping_add(1);
            self.last_reject = Some(fault);
            return false;
        }
        self.accept_source(stamp.source_id)
            && self.accept_incarnation(stamp.incarnation)
            && self.accept_epoch(stamp.epoch)
    }

    /// Per-stream ordering ladder against the current slot, if any.
    fn admit_ordering(&mut self, stamp: &RawStamp, current: Option<RawStamp>) -> bool {
        let Some(current) = current else {
            return true;
        };
        if stamp.sequence == current.sequence {
            self.counters.duplicates = self.counters.duplicates.wrapping_add(1);
            return false;
        }
        if !serial_is_newer(stamp.sequence, current.sequence) {
            self.counters.reordered = self.counters.reordered.wrapping_add(1);
            return false;
        }
        if stamp.clock != current.clock {
            self.counters.clock_changes = self.counters.clock_changes.wrapping_add(1);
            return false;
        }
        if stamp.acquired_at_ns <= current.acquired_at_ns {
            self.counters.time_regressions = self.counters.time_regressions.wrapping_add(1);
            return false;
        }
        let gap = serial_distance(stamp.sequence, current.sequence);
        if gap > 1 {
            self.counters.sequence_gaps = self.counters.sequence_gaps.wrapping_add(gap - 1);
        }
        true
    }

    fn accept_source(&mut self, candidate: u64) -> bool {
        match self.source_id {
            None => {
                self.source_id = Some(candidate);
                true
            }
            Some(pinned) if pinned == candidate => true,
            Some(_) => {
                self.counters.wrong_source = self.counters.wrong_source.wrapping_add(1);
                false
            }
        }
    }

    fn accept_incarnation(&mut self, candidate: [u8; 16]) -> bool {
        match self.incarnation {
            None => {
                self.incarnation = Some(candidate);
                self.remember_incarnation(candidate);
                true
            }
            Some(pinned) if pinned == candidate => true,
            Some(_) => self.accept_incarnation_transition(candidate),
        }
    }

    fn accept_incarnation_transition(&mut self, candidate: [u8; 16]) -> bool {
        if self.seen.iter().flatten().any(|seen| *seen == candidate) {
            self.counters.old_incarnation = self.counters.old_incarnation.wrapping_add(1);
            return false;
        }
        if self.config.incarnation_policy != IncarnationPolicy::SimAcceptUnseen {
            self.counters.wrong_incarnation = self.counters.wrong_incarnation.wrapping_add(1);
            return false;
        }
        let capacity = self
            .config
            .maximum_seen_incarnations
            .min(MAX_SEEN_INCARNATIONS);
        if self.seen.iter().flatten().count() >= capacity {
            self.counters.incarnation_capacity = self.counters.incarnation_capacity.wrapping_add(1);
            return false;
        }
        self.remember_incarnation(candidate);
        self.incarnation = Some(candidate);
        self.epoch = None;
        self.clear_admitted_state();
        self.last_coherence = Coherence::Insufficient;
        self.counters.incarnation_transitions =
            self.counters.incarnation_transitions.wrapping_add(1);
        true
    }

    fn accept_epoch(&mut self, candidate: u32) -> bool {
        match self.epoch {
            None => {
                self.epoch = Some(candidate);
                true
            }
            Some(current) if current == candidate => true,
            Some(current) => {
                if !serial_is_newer(candidate, current) {
                    self.counters.old_epoch = self.counters.old_epoch.wrapping_add(1);
                    return false;
                }
                self.epoch = Some(candidate);
                self.clear_admitted_state();
                self.counters.source_resets = self.counters.source_resets.wrapping_add(1);
                true
            }
        }
    }

    fn remember_incarnation(&mut self, candidate: [u8; 16]) {
        if let Some(slot) = self.seen.iter_mut().find(|slot| slot.is_none()) {
            *slot = Some(candidate);
        }
    }

    fn clear_admitted_state(&mut self) {
        self.attitude = None;
        self.kinematics = None;
        self.estimator_status = None;
        self.regime = None;
        self.previous_regime = None;
        self.fail_closed_authorization();
    }

    fn fail_closed_authorization(&mut self) {
        self.attitude_paired = false;
        self.kinematics_paired = false;
        self.valid_flags = 0;
        self.quality = QUALITY_UNUSABLE;
    }

    fn record_coherence_transition(&mut self) {
        let coherence = self.coherence();
        if coherence.status == Coherence::ExcessiveSkew
            && self.last_coherence != Coherence::ExcessiveSkew
        {
            self.counters.excessive_skew = self.counters.excessive_skew.wrapping_add(1);
        }
        self.last_coherence = coherence.status;
    }

    fn coherence(&self) -> CoherenceReport {
        let (Some(attitude), Some(kinematics)) = (&self.attitude, &self.kinematics) else {
            return CoherenceReport::default();
        };
        let a = &attitude.stamp;
        let k = &kinematics.stamp;
        if !a.same_stream(k) {
            return CoherenceReport::default();
        }
        let skew = skew_ns(a.acquired_at_ns, k.acquired_at_ns);
        CoherenceReport {
            status: if skew <= self.config.maximum_skew_nanos {
                Coherence::Coherent
            } else {
                Coherence::ExcessiveSkew
            },
            skew_nanos: Some(skew),
        }
    }

    /// The current admitted state; ages against the caller's clock.
    pub fn snapshot(&self, now_ms: f64) -> IngressSnapshot {
        let age = |accepted_at_ms: f64| (now_ms - accepted_at_ms).max(0.0);
        IngressSnapshot {
            generation: self.generation,
            source_id: self.source_id,
            incarnation: self.incarnation,
            epoch: self.epoch,
            attitude: self.attitude.map(|group| GroupSnapshot {
                data: group.data,
                stamp: group.stamp,
                age_ms: age(group.accepted_at_ms),
            }),
            kinematics: self.kinematics.map(|group| GroupSnapshot {
                data: group.data,
                stamp: group.stamp,
                age_ms: age(group.accepted_at_ms),
            }),
            estimator_status: self.estimator_status.map(|group| GroupSnapshot {
                data: (),
                stamp: group.stamp,
                age_ms: age(group.accepted_at_ms),
            }),
            valid_flags: self.valid_flags,
            quality: self.quality,
            coherence: self.coherence(),
        }
    }

    /// Refusal counters plus the last stamp fault.
    pub fn diagnostics(&self) -> (IngressCounters, Option<StampFault>) {
        (self.counters, self.last_reject)
    }
}

mod authorization;

#[cfg(test)]
mod tests;
