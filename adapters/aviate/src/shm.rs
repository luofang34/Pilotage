//! Thin safe wrapper around the Aviate XIL shared-memory consumer endpoint
//! (ADR-0019: the co-located SITL vehicle link binds shared memory).
//!
//! The contract layout, attach validation (magic / layout version /
//! declared size / readiness), the seqlock read protocol, and writer
//! liveness/replacement detection are owned by the upstream
//! `aviate-xil-contract` + `aviate-xil-shm` crates, pinned to one exact
//! revision — this module carries no layout knowledge of its own. It
//! resolves the canonical object name from the upstream naming authority,
//! converts each coherent snapshot at the boundary (world ENU → NED,
//! ENU/FLU quaternion → NED/FRD), and tracks wall-clock freshness so a
//! frozen block ages into withheld telemetry instead of replaying.
//!
//! The block is coherent simulator ground truth, not an FC operational
//! estimate. It is published only as the typed simulation-truth oracle
//! (`SimTruthSample`) under its own source identity: it drives no
//! primary panel, never seeds command construction, and is not a
//! fallback for a missing FC estimate — a session without an estimate
//! rejects state-dependent control rather than borrowing truth.
//!
//! The contract's angular-velocity lane is world-frame and advisory
//! (gz's `WorldAngularVelocity` verbatim; zero on the known setup). It is
//! NOT a body gyro, so no body-rate field exists on [`GzStateSample`]:
//! consumers needing body rates must derive them from successive
//! attitudes and label them as derived.

use std::time::Instant;

use aviate_xil_contract::{ShmName, WriterState, shm_name};
use aviate_xil_shm::{AttachFailure, ConsumerSession, ModelStateSnapshot};

use crate::error::AviateAdapterError;

/// One coherent ground-truth sample, already in NED/FRD.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GzStateSample {
    /// Attitude quaternion (w, x, y, z), body FRD → world NED.
    pub quat_wxyz: [f32; 4],
    /// Position NED, meters.
    pub pos_ned_m: [f32; 3],
    /// Velocity NED, m/s.
    pub vel_ned_mps: [f32; 3],
    /// Simulation time in microseconds. Rewinds to zero on a world reset;
    /// pair with `reset_generation` before judging progress.
    pub time_us: u64,
    /// Physics step counter, the source sequence: monotonic across world
    /// resets (epochs are told apart by `reset_generation`, not by this
    /// counter restarting).
    pub sim_step: u64,
    /// The simulation-world epoch this snapshot belongs to, coherent with
    /// the payload it stamps.
    pub reset_generation: u32,
}

/// A read-only attachment to the Aviate gz-bridge block.
#[derive(Debug)]
pub struct GzStateShm {
    session: ConsumerSession,
}

impl GzStateShm {
    /// Attaches read-only to instance `instance`'s block, named by the
    /// upstream naming authority ([`shm_name`]).
    ///
    /// # Errors
    ///
    /// Returns a typed [`AviateAdapterError`] when the object is absent or
    /// unmappable ([`AviateAdapterError::ShmAttachIo`]), carries a foreign
    /// or stale contract ([`AviateAdapterError::ShmContractMismatch`]), or
    /// has no ready writer ([`AviateAdapterError::ShmWriterNotReady`]).
    pub fn open(instance: u8) -> Result<Self, AviateAdapterError> {
        Self::open_named(&object_name(instance))
    }

    /// Attaches read-only to the POSIX shm object `name`. [`Self::open`]
    /// resolves the production name; tests attach to a private object.
    pub(crate) fn open_named(name: &str) -> Result<Self, AviateAdapterError> {
        match ConsumerSession::attach(name) {
            Ok(session) => Ok(Self { session }),
            Err(failure) => Err(attach_error(name, failure)),
        }
    }

    /// What the object name behind this attachment resolves to right now.
    /// [`WriterState::Current`] is the only state in which reads are
    /// trustworthy; every other state is the sampler's signal to stop
    /// output and re-attach or fail closed.
    pub fn writer_state(&self) -> WriterState {
        self.session.writer_state()
    }

    /// Reads one coherent sample, or `None` when no snapshot is published
    /// (writer mid-initialization, world mid-reset, or a retired epoch).
    /// `None` means publish nothing — never replay a cached sample.
    pub fn read(&self) -> Option<GzStateSample> {
        self.session
            .read_model_state()
            .map(|snapshot| sample_from_snapshot(&snapshot))
    }
}

/// Canonical POSIX shm object name for one simulator instance, derived
/// from the upstream naming authority — never assembled here, so the
/// versioned namespace and its guardrails stay upstream's alone.
pub(crate) fn object_name(instance: u8) -> ShmName {
    shm_name(u32::from(instance))
}

/// Maps an upstream attach refusal into the adapter's typed error,
/// preserving the original context (never collapsed or discarded).
pub(crate) fn attach_error(name: &str, failure: AttachFailure) -> AviateAdapterError {
    let name = name.to_owned();
    match failure {
        AttachFailure::Io(source) => AviateAdapterError::ShmAttachIo { name, source },
        AttachFailure::Contract(violation) => {
            AviateAdapterError::ShmContractMismatch { name, violation }
        }
        AttachFailure::NotReady => AviateAdapterError::ShmWriterNotReady { name },
    }
}

/// Converts one upstream snapshot at the adapter boundary: world ENU →
/// NED, ENU/FLU quaternion → NED/FRD; `time_us`, `sim_step`, and
/// `reset_generation` pass through unchanged. The advisory world-frame
/// `ang_vel` lane is deliberately dropped (see the module docs).
pub(crate) fn sample_from_snapshot(snapshot: &ModelStateSnapshot) -> GzStateSample {
    GzStateSample {
        quat_wxyz: enu_quat_to_ned(snapshot.quat),
        pos_ned_m: enu_to_ned(snapshot.pos),
        vel_ned_mps: enu_to_ned(snapshot.vel),
        time_us: snapshot.time_us,
        sim_step: snapshot.sim_step,
        reset_generation: snapshot.reset_generation,
    }
}

/// ENU world vector → NED (swap x/y, negate z), matching Aviate's
/// `enu_to_ned_f32`.
fn enu_to_ned(enu: [f64; 3]) -> [f32; 3] {
    [enu[1] as f32, enu[0] as f32, -enu[2] as f32]
}

/// Body→world quaternion, ENU/FLU convention → NED/FRD, matching
/// Aviate's `enu_quat_to_ned_f32`:
/// `q_NED_FRD = q_ENU→NED · q_ENU_FLU · q_FRD→FLU`.
fn enu_quat_to_ned(q: [f64; 4]) -> [f32; 4] {
    let s = core::f32::consts::FRAC_1_SQRT_2;
    let (w, x, y, z) = (q[0] as f32, q[1] as f32, q[2] as f32, q[3] as f32);
    [s * (w + z), s * (x + y), s * (x - y), s * (w - z)]
}

/// Progress classification for one coherent SHM observation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShmObservation {
    /// Step counter and simulation time advanced within the same epoch.
    Advancing,
    /// The same coherent sample remains published, with its frozen duration.
    Unchanged(std::time::Duration),
    /// Step counter or simulation time rolled back within the same epoch —
    /// a protocol violation the writer-state machine did not announce.
    Quarantined,
}

/// Wall-clock progress tracking so a frozen block (paused sim, stalled
/// plugin) ages into withheld telemetry instead of replaying forever.
/// Tracks progress WITHIN one world epoch of one attachment; a
/// `reset_generation` change or a re-attachment is the caller's signal to
/// re-baseline with a fresh tracker (sim time rewinds to zero by design
/// on a world reset).
#[derive(Debug)]
pub struct ShmFreshness {
    last_step: Option<u64>,
    last_time_us: Option<u64>,
    last_progress: Instant,
    quarantined: bool,
}

impl ShmFreshness {
    /// Starts tracking.
    pub fn new() -> Self {
        Self::new_at(Instant::now())
    }

    pub(crate) fn new_at(now: Instant) -> Self {
        Self {
            last_step: None,
            last_time_us: None,
            last_progress: now,
            quarantined: false,
        }
    }

    /// Feeds the latest observed `sim_step`/`time_us` pair and classifies
    /// the block's progress.
    pub fn observe(&mut self, sim_step: u64, time_us: u64) -> ShmObservation {
        self.observe_at(sim_step, time_us, Instant::now())
    }

    pub(crate) fn observe_at(
        &mut self,
        sim_step: u64,
        time_us: u64,
        now: Instant,
    ) -> ShmObservation {
        if self.quarantined {
            return ShmObservation::Quarantined;
        }
        let observation = match (self.last_step, self.last_time_us) {
            (Some(previous_step), Some(previous_time))
                if sim_step == previous_step && time_us == previous_time =>
            {
                ShmObservation::Unchanged(
                    now.checked_duration_since(self.last_progress)
                        .unwrap_or_default(),
                )
            }
            (Some(previous_step), Some(previous_time))
                if serial_is_newer(sim_step, previous_step) && time_us > previous_time =>
            {
                ShmObservation::Advancing
            }
            (None, None) => ShmObservation::Advancing,
            _ => {
                self.quarantined = true;
                ShmObservation::Quarantined
            }
        };
        if observation == ShmObservation::Advancing {
            self.last_step = Some(sim_step);
            self.last_time_us = Some(time_us);
            self.last_progress = now;
        }
        observation
    }
}

fn serial_is_newer(candidate: u64, current: u64) -> bool {
    let distance = candidate.wrapping_sub(current);
    distance != 0 && distance < (1_u64 << 63)
}

impl Default for ShmFreshness {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests;
