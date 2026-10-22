//! The fixed outputs of a tick: terminal states, a failed core, and the
//! zero-velocity intent that hands a completed plan to the adapter's hold.

use pilotage_protocol::{ControlIntent, ReferenceFrame, VelocityIntent};

use super::super::{MissionEvent, MissionOutput, MissionState};

pub(super) fn terminal_output(state: MissionState) -> MissionOutput {
    MissionOutput {
        intent: (state == MissionState::Complete).then(zero_velocity_intent),
        action: None,
        state,
        events: Vec::new(),
    }
}

pub(super) fn failed_output(events: Vec<MissionEvent>) -> MissionOutput {
    MissionOutput {
        intent: None,
        action: None,
        state: MissionState::Failed,
        events,
    }
}

pub(super) fn zero_velocity_intent() -> ControlIntent {
    ControlIntent::Velocity(VelocityIntent {
        frame: ReferenceFrame::BodyFrd,
        vx: 0.0,
        vy: 0.0,
        vz: 0.0,
        yaw_rate: 0.0,
    })
}
