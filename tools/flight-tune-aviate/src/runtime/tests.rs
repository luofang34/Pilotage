//! Unit tests for the production Aviate runtime.

#![allow(clippy::expect_used, clippy::panic)]

mod conditions;
mod publication;
mod telemetry;
mod terminal;
mod timing;
mod waveform;

use flight_tune::{ArtifactIdentity, Digest, KinematicTruth, ScenarioFrame};

use super::timing::FrameStamp;

/// One artifact identity with a distinct, non-zero digest.
fn identity(id: &str, fill: u8) -> ArtifactIdentity {
    ArtifactIdentity::new(id, Digest::from_bytes([fill; 32])).expect("a named test identity")
}

/// One frame at rest, at the requested sequence and trial time.
fn frame(source_sequence: u64, trial_time_ns: u64) -> ScenarioFrame {
    ScenarioFrame {
        source_sequence,
        simulator_time_ns: trial_time_ns,
        trial_time_ns,
        lifecycle: None,
        ground_contact: Some(false),
        crashed: Some(false),
        link_valid: Some(true),
        estimator_valid: Some(true),
        truth: KinematicTruth {
            position_ned_m: [0.0; 3],
            velocity_ned_mps: [0.0; 3],
            acceleration_ned_mps2: [0.0; 3],
            attitude_wxyz: [1.0, 0.0, 0.0, 0.0],
            body_rates_rps: [0.0; 3],
        },
        applied_conditions: std::collections::BTreeMap::new(),
        canonical_signals: Vec::new(),
    }
}

/// The stamp one frame carries onto the sample clock.
const fn stamp(source_sequence: u64, trial_time_ns: u64) -> FrameStamp {
    FrameStamp {
        source_sequence,
        simulator_time_ns: trial_time_ns,
        trial_time_ns,
    }
}
