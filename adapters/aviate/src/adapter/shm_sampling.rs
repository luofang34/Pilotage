//! Shared-memory source sampling: the consumer-session state machine and
//! source-epoch transitions.
//!
//! One explicit machine over the upstream `writer_state()`:
//!
//! * `Current` — the only state that publishes samples.
//! * `Replaced` / `Gone` / `Initializing` — output stops immediately and
//!   the source re-attaches on a bounded interval; a successful re-attach
//!   starts a new Pilotage source epoch with fresh freshness state.
//! * `ContractMismatch` — the source fails closed with a sticky typed
//!   fault; no sample is ever read through a foreign layout.
//!
//! An unpublished snapshot (`read` returning `None`) publishes nothing:
//! frozen data is never replayed. A `reset_generation` change on the same
//! writer starts a new source epoch and accepts the simulation-time
//! rewind.

use std::time::{Duration, Instant};

use aviate_xil_contract::WriterState;
use pilotage_adapter_api::{
    AvionicsAttitudeSample, AvionicsKinematicsSample, AvionicsSample, MeasurementClock,
    MeasurementStamp, Pose2d, SourceIncarnation, TelemetryBatch, TelemetrySample,
};
use pilotage_protocol::VehicleId;
use pilotage_timing::SimTick;

use super::{WITHHOLD_AFTER, yaw_of};
use crate::error::AviateAdapterError;
use crate::shm::{GzStateSample, GzStateShm, ShmFreshness, ShmObservation, object_name};

const REATTACH_INTERVAL: Duration = Duration::from_millis(250);

/// Simulator-field availability, in the wire batch's flag positions:
/// attitude (bit 0), position (bit 2), and velocity (bit 3) are present.
/// The rates bit (1) stays CLEAR: the contract carries no body gyro — its
/// angular-velocity lane is world-frame and advisory — so body rates are
/// published unavailable. This says which simulator fields exist; it is
/// NOT an FC-estimator authorization.
const TRUTH_VALID_FLAGS: u32 = 0b1101;

#[derive(Debug)]
pub(super) struct ShmSource {
    /// The live consumer attachment; `None` while detached (writer gone,
    /// replaced, or mid-initialization) and awaiting re-attachment.
    session: Option<GzStateShm>,
    /// Sticky fail-closed fault: set on a contract mismatch, never
    /// cleared. While present, no sample is published and no
    /// re-attachment is attempted.
    fault: Option<AviateAdapterError>,
    freshness: ShmFreshness,
    name: String,
    instance: u8,
    epoch: u32,
    incarnation: SourceIncarnation,
    last_reattach_attempt: Option<Instant>,
    reset_generation: Option<u32>,
}

impl ShmSource {
    pub(super) fn open(
        instance: u8,
        incarnation: SourceIncarnation,
    ) -> Result<Self, AviateAdapterError> {
        Self::open_named(&object_name(instance), instance, incarnation)
    }

    /// Attaches to an explicit object name. [`Self::open`] resolves the
    /// canonical production name; tests attach to a private object.
    fn open_named(
        name: &str,
        instance: u8,
        incarnation: SourceIncarnation,
    ) -> Result<Self, AviateAdapterError> {
        Ok(Self {
            session: Some(GzStateShm::open_named(name)?),
            fault: None,
            freshness: ShmFreshness::new(),
            name: name.to_owned(),
            instance,
            epoch: 1,
            incarnation,
            last_reattach_attempt: None,
            reset_generation: None,
        })
    }

    pub(super) fn current_pose(&mut self) -> Option<(f32, [f32; 3])> {
        self.usable_sample(Instant::now())
            .map(|sample| (yaw_of(sample.quat_wxyz) as f32, sample.pos_ned_m))
    }

    pub(super) fn tick(&self) -> u64 {
        self.session
            .as_ref()
            .filter(|session| session.writer_state() == WriterState::Current)
            .and_then(GzStateShm::read)
            .map_or(0, |sample| sample.time_us.wrapping_mul(1_000))
    }

    /// The typed fault that has fail-closed this source, if any.
    pub(super) fn fault(&self) -> Option<&AviateAdapterError> {
        self.fault.as_ref()
    }

    pub(super) fn sample(&mut self, vehicle: VehicleId, arm_state: u32) -> TelemetryBatch {
        let now = Instant::now();
        let Some(sample) = self.usable_sample(now) else {
            return TelemetryBatch::default();
        };
        batch_from_sample(
            vehicle,
            arm_state,
            sample,
            self.instance,
            self.epoch,
            self.incarnation,
        )
    }

    fn usable_sample(&mut self, now: Instant) -> Option<GzStateSample> {
        if self.fault.is_some() {
            return None;
        }
        match self.session.as_ref().map(GzStateShm::writer_state) {
            Some(WriterState::Current) => return self.read_current(now),
            Some(state) => {
                // Replaced, Gone, Initializing, or ContractMismatch: this
                // mapping serves a dead (or not-yet-born, or foreign)
                // world. Output stops in this same call; the re-attach
                // below decides between a new epoch and a fail-closed
                // fault.
                tracing::warn!(?state, "Aviate shm writer lost; output stopped");
                self.session = None;
                self.last_reattach_attempt = None;
            }
            None => {}
        }
        self.try_reattach(now);
        // A successful re-attach serves its first coherent sample in the
        // same call, so the new epoch starts with data, not a blank tick.
        if self.session.is_some() {
            self.read_current(now)
        } else {
            None
        }
    }

    /// Reads through a `Current` writer. `None` from the upstream read —
    /// writer mid-initialization, world mid-reset, or a retired epoch —
    /// publishes nothing; frozen data is never replayed.
    fn read_current(&mut self, now: Instant) -> Option<GzStateSample> {
        let sample = self.session.as_ref().and_then(GzStateShm::read)?;
        if self
            .reset_generation
            .is_some_and(|generation| generation != sample.reset_generation)
        {
            // A reset_generation change is a world reset on the SAME
            // writer: sim time rewinds to zero by design. Re-baseline
            // freshness in a fresh source epoch instead of quarantining,
            // so telemetry survives a world reset without a host restart.
            self.freshness = ShmFreshness::new_at(now);
            self.epoch = self.epoch.wrapping_add(1);
            tracing::info!(
                reset_generation = sample.reset_generation,
                source_epoch = self.epoch,
                "Aviate world reset observed; shm freshness re-baselined"
            );
        }
        self.reset_generation = Some(sample.reset_generation);
        match self
            .freshness
            .observe_at(sample.sim_step, sample.time_us, now)
        {
            ShmObservation::Advancing => Some(sample),
            ShmObservation::Unchanged(age) if age <= WITHHOLD_AFTER => Some(sample),
            ShmObservation::Unchanged(_) | ShmObservation::Quarantined => None,
        }
    }

    /// One bounded re-attachment attempt: rate-limited to
    /// [`REATTACH_INTERVAL`], never spinning on an absent writer. A
    /// contract mismatch fails the source closed with the upstream
    /// refusal preserved verbatim.
    fn try_reattach(&mut self, now: Instant) {
        if self.last_reattach_attempt.is_some_and(|last| {
            now.checked_duration_since(last).unwrap_or_default() < REATTACH_INTERVAL
        }) {
            return;
        }
        self.last_reattach_attempt = Some(now);
        self.absorb_reattach(GzStateShm::open_named(&self.name), now);
    }

    /// Applies one re-attachment outcome to the machine: a new session
    /// starts a new source epoch; a contract mismatch is a sticky
    /// fail-closed fault; any other refusal leaves the source detached
    /// until the next bounded attempt.
    fn absorb_reattach(&mut self, attach: Result<GzStateShm, AviateAdapterError>, now: Instant) {
        match attach {
            Ok(session) => {
                self.session = Some(session);
                self.freshness = ShmFreshness::new_at(now);
                self.epoch = self.epoch.wrapping_add(1);
                // A different object is a different writer; its generation
                // counter starts a new numbering and must not be compared
                // with the old one.
                self.reset_generation = None;
                self.last_reattach_attempt = None;
                tracing::warn!(
                    source_epoch = self.epoch,
                    "Aviate SHM object entered a new attachment epoch"
                );
            }
            Err(error @ AviateAdapterError::ShmContractMismatch { .. }) => {
                // A foreign or stale layout stands behind the canonical
                // name: reading it would be plausible garbage. Fail closed
                // and stay closed.
                tracing::error!(%error, "Aviate shm contract mismatch; source failed closed");
                self.fault = Some(error);
            }
            Err(_) => {
                // Absent or not-yet-ready writer: retry at the next
                // interval without replaying anything meanwhile.
            }
        }
    }
}

fn batch_from_sample(
    vehicle: VehicleId,
    arm_state: u32,
    sample: GzStateSample,
    instance: u8,
    epoch: u32,
    incarnation: SourceIncarnation,
) -> TelemetryBatch {
    let heading = yaw_of(sample.quat_wxyz);
    let stamp = MeasurementStamp {
        source_id: u64::from(instance).wrapping_add(1),
        source_incarnation: incarnation,
        source_epoch: epoch,
        // The stamp carries a wrapping u32 group sequence; the physics
        // step counter's low 32 bits preserve adjacency and wrap
        // semantics.
        sequence: sample.sim_step as u32,
        acquired_at_ns: sample.time_us.wrapping_mul(1_000),
        clock: MeasurementClock::Simulation,
    };
    let speed = f64::from(
        (sample.vel_ned_mps[0] * sample.vel_ned_mps[0]
            + sample.vel_ned_mps[1] * sample.vel_ned_mps[1])
            .sqrt(),
    );
    TelemetryBatch {
        samples: vec![TelemetrySample {
            vehicle,
            tick: SimTick::new(sample.time_us.wrapping_mul(1_000)),
            pose: Some(Pose2d {
                x: f64::from(sample.pos_ned_m[0]),
                y: f64::from(sample.pos_ned_m[1]),
                heading,
            }),
            speed: Some(speed),
            avionics: Some(AvionicsSample {
                attitude: Some(AvionicsAttitudeSample {
                    quat_wxyz: sample.quat_wxyz,
                    // No body gyro exists on this source (see the
                    // `crate::shm` docs): rates are neutral with their
                    // validity bit clear in `TRUTH_VALID_FLAGS`.
                    rates_rps: [0.0; 3],
                    stamp,
                }),
                kinematics: Some(AvionicsKinematicsSample {
                    pos_ned_m: sample.pos_ned_m,
                    vel_ned_mps: sample.vel_ned_mps,
                    stamp,
                }),
                // COMPATIBILITY PROJECTION: the wire batch has one
                // estimator-shaped avionics sample, so simulator truth
                // rides it with the stamp marking simulator-field
                // availability — not an FC estimator's authorization.
                // The truth-versus-estimate source split is a separate
                // concern, not solved by this projection.
                estimator_status_stamp: Some(stamp),
                valid_flags: TRUTH_VALID_FLAGS,
                quality: 0,
                arm_state,
            }),
        }],
    }
}

#[cfg(test)]
mod tests;
