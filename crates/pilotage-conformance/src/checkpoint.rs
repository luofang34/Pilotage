//! Golden exact-equality trajectory checkpoints for the increment-0 fixture
//! (ADR-0008 deterministic conformance anchor, ADR-0012 replay).
//!
//! The reference adapter routes all trigonometry through a portable `libm`
//! specifically so its `f64` trajectory is bit-identical on every machine
//! (see `adapters/reference-headless`). That lets these checkpoints assert
//! exact `f64` equality rather than an epsilon tolerance: a checkpoint is
//! stored as the raw IEEE-754 bit pattern of each field, so an accidental
//! change to the physics, the seed handling, or the control pipeline is
//! caught immediately instead of drifting silently.
//!
//! The values were computed once by running the fixture's exact apply/step
//! sequence against the reference adapter and recording the resulting
//! telemetry bits; [`crate::increment_zero_script`] and these checkpoints
//! are kept in lockstep by the conformance tests.

use pilotage_adapter_api::{TelemetrySample, VehicleAdapter};
use pilotage_adapter_reference::ReferenceAdapter;
use pilotage_timing::SimTick;

/// A golden telemetry checkpoint: the simulation tick and the exact `f64`
/// bit patterns of pose and speed the adapter must report there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TrajectoryCheckpoint {
    /// A human-readable label naming the session phase this checkpoint ends.
    pub label: &'static str,
    /// Simulation tick the adapter must be at.
    pub tick: u64,
    /// Exact IEEE-754 bits of the reported x position.
    pub x_bits: u64,
    /// Exact IEEE-754 bits of the reported y position.
    pub y_bits: u64,
    /// Exact IEEE-754 bits of the reported heading.
    pub heading_bits: u64,
    /// Exact IEEE-754 bits of the reported scalar speed.
    pub speed_bits: u64,
}

impl TrajectoryCheckpoint {
    /// Returns `true` when `sample` matches this checkpoint's tick and every
    /// pose/speed field bit-for-bit.
    #[must_use]
    pub fn matches(&self, sample: &TelemetrySample) -> bool {
        let (Some(pose), Some(speed)) = (sample.pose, sample.speed) else {
            return false;
        };
        sample.tick == SimTick::new(self.tick)
            && pose.x.to_bits() == self.x_bits
            && pose.y.to_bits() == self.y_bits
            && pose.heading.to_bits() == self.heading_bits
            && speed.to_bits() == self.speed_bits
    }

    /// Captures a checkpoint from the adapter's current telemetry, for
    /// regenerating golden values or asserting against a live run.
    #[must_use]
    pub fn capture(label: &'static str, adapter: &mut ReferenceAdapter) -> Option<Self> {
        let batch = adapter.sample_telemetry();
        let sample = batch.samples.first()?;
        let pose = sample.pose?;
        let speed = sample.speed?;
        Some(Self {
            label,
            tick: sample.tick.as_u64(),
            x_bits: pose.x.to_bits(),
            y_bits: pose.y.to_bits(),
            heading_bits: pose.heading.to_bits(),
            speed_bits: speed.to_bits(),
        })
    }
}

/// The ordered golden checkpoints for the increment-0 fixture, one per
/// stepped phase (A drives, B drives after handover, C drives after
/// override, and the neutralized link-loss decay).
///
/// A neutralized adapter still moves — position keeps integrating the
/// residual speed — but the speed strictly decays under drag once controls
/// are zeroed, so `link_loss_neutralize`'s speed is below `override_c`'s.
#[must_use]
pub fn increment_zero_checkpoints() -> [TrajectoryCheckpoint; 4] {
    [
        TrajectoryCheckpoint {
            label: "grant_a_drives",
            tick: 10,
            x_bits: 0x4007_254d_5b71_26f4,
            y_bits: 0x4010_ff67_0d37_893f,
            heading_bits: 0x400b_40dc_1ae4_74c7,
            speed_bits: 0x3fd9_0817_cd82_abbc,
        },
        TrajectoryCheckpoint {
            label: "handover_b_drives",
            tick: 20,
            x_bits: 0x4006_bcad_9f5c_9416,
            y_bits: 0x4010_f345_bff7_122d,
            heading_bits: 0x400a_a742_814a_db31,
            speed_bits: 0x3fe5_eaa3_67f4_2b13,
        },
        TrajectoryCheckpoint {
            label: "override_c_drives",
            tick: 30,
            x_bits: 0x4006_1246_6b6f_8073,
            y_bits: 0x4010_e2e1_336d_4727,
            heading_bits: 0x400a_a742_814a_db31,
            speed_bits: 0x3ff0_ae30_d25f_09bf,
        },
        TrajectoryCheckpoint {
            label: "link_loss_neutralize",
            tick: 40,
            x_bits: 0x4005_4544_195f_cfd0,
            y_bits: 0x4010_cf28_67c9_adb7,
            heading_bits: 0x400a_a742_814a_db31,
            speed_bits: 0x3fef_bad7_e1a1_d4eb,
        },
    ]
}
