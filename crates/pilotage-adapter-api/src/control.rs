//! Control-application outcomes and link-loss policy vocabulary (ADR-0008).

use pilotage_protocol::{ButtonEdge, ControlAction, ControlIntent, ControlPayload, LogicalAxisId};
use pilotage_timing::SimTick;
use serde::{Deserialize, Serialize};

use crate::capability::IntentCapability;

/// Whether a control payload proves neutral activation for every axis a
/// scope declares.
///
/// Every declared axis must be reported inside the deadband, every reported
/// axis must be finite and inside it, and no pressed button edge is allowed.
/// Full coverage prevents a retained value for an omitted axis from becoming
/// active when a safety latch clears.
#[must_use]
pub fn payload_satisfies_neutral_activation(
    payload: &ControlPayload,
    declared_axes: &[LogicalAxisId],
    deadband_milli: u32,
) -> bool {
    let deadband = deadband_milli as f32 / 1000.0;
    let all_declared_reported_neutral = declared_axes.iter().all(|axis| {
        payload
            .axes
            .iter()
            .any(|(reported, value)| reported == axis && value.abs() <= deadband)
    });
    let all_reported_neutral = payload
        .axes
        .iter()
        .all(|(_, value)| value.abs() <= deadband);
    let no_pressed_edge = payload
        .edges
        .iter()
        .all(|(_, edge)| *edge != ButtonEdge::Pressed);
    all_declared_reported_neutral && all_reported_neutral && no_pressed_edge
}

/// Why an adapter did not accept a control frame as-is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RejectReason {
    /// The frame targeted a scope the vehicle does not expose.
    UnknownScope,
    /// The frame targeted a logical axis the scope does not accept.
    UnknownAxis,
    /// The frame targeted a vehicle the adapter does not know.
    UnknownVehicle,
    /// The frame failed a fencing check (stale generation or sequence).
    Fenced,
    /// A measurement required to apply the frame is unavailable.
    MeasurementUnavailable,
    /// A link-loss policy is engaged on the vehicle; control frames are
    /// suppressed until the policy is cleared through the host's recovery
    /// path (a fresh authority generation plus the scope's activation
    /// condition, ADR-0008). Without this latch a newly granted holder
    /// with deflected sticks would drive the vehicle straight out of its
    /// neutralized state.
    LinkLossEngaged,
    /// A commanded simulation reset is in progress on the vehicle:
    /// control frames are suppressed until the estimate stream provably
    /// restarts (a fresh source epoch) and the holder demonstrates
    /// neutral input. Without this latch, an arm pressed inside the
    /// restart window would be validated against pre-reset measurements
    /// still within the freshness budget, and could reach the rebooting
    /// FC while its estimator is unconverged. Disarm is exempt:
    /// surrendering authority is never blocked.
    ResetInProgress,
    /// The adapter rejected the frame for a reason not covered above.
    Other(String),
}

/// How an adapter disposed of an applied control frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Disposition {
    /// The frame was applied exactly as received.
    Accepted,
    /// The frame was applied after the adapter transformed it (e.g.
    /// clamping to a physical limit).
    Transformed,
    /// The frame was constrained by a safety or authority rule and only
    /// partially applied.
    Constrained,
    /// The frame was not applied.
    Rejected(RejectReason),
}

/// The explicit outcome of ONE typed discrete action an adapter disposed of:
/// the session host forwards each as a `ControlActionResult` to the sender,
/// so a press is never silently dropped (CTRL-01).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionResult {
    /// The action this result answers.
    pub action: ControlAction,
    /// Whether the adapter executed (or queued to the vehicle link) the
    /// action.
    pub accepted: bool,
    /// Adapter-supplied reason when not accepted; empty on acceptance.
    pub detail: String,
}

impl ActionResult {
    /// An accepted action.
    #[must_use]
    pub const fn accepted(action: ControlAction) -> Self {
        Self {
            action,
            accepted: true,
            detail: String::new(),
        }
    }

    /// A rejected action with its reason.
    #[must_use]
    pub fn rejected(action: ControlAction, detail: impl Into<String>) -> Self {
        Self {
            action,
            accepted: false,
            detail: detail.into(),
        }
    }
}

/// The result of applying a single control frame (ADR-0008): the simulation
/// tick the outcome corresponds to, how the frame was disposed of, and the
/// per-action outcomes for every typed discrete action it carried.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplyOutcome {
    /// Simulation tick this outcome corresponds to.
    pub tick: SimTick,
    /// How the frame was disposed of.
    pub disposition: Disposition,
    /// One explicit result per typed discrete action the frame carried
    /// (empty for a frame without actions, or one rejected whole — the
    /// host answers a rejected frame with `FrameRejected` instead).
    pub action_results: Vec<ActionResult>,
}

impl ApplyOutcome {
    /// An outcome with no per-action results.
    #[must_use]
    pub const fn new(tick: SimTick, disposition: Disposition) -> Self {
        Self {
            tick,
            disposition,
            action_results: Vec::new(),
        }
    }
}

/// Whether a typed intent proves neutral activation against its advertised
/// capability: every velocity or rate component must sit inside the
/// limit-scaled deadband. Families without a meaningful neutral posture
/// (attitude, position hold, body rate) never satisfy it — recovery
/// activation stays fail-closed for them.
#[must_use]
pub fn intent_satisfies_neutral_activation(
    intent: &ControlIntent,
    capability: &IntentCapability,
    deadband_milli: u32,
) -> bool {
    let fraction = deadband_milli as f32 / 1000.0;
    match intent {
        ControlIntent::Velocity(v) => {
            let linear = capability.max_linear * fraction;
            let vertical = capability.effective_vertical() * fraction;
            let angular = capability.max_angular * fraction;
            v.vx.abs() <= linear
                && v.vy.abs() <= linear
                && v.vz.abs() <= vertical
                && v.yaw_rate.abs() <= angular
        }
        ControlIntent::GimbalRate(g) => {
            let angular = capability.max_angular * fraction;
            g.pitch_rate.abs() <= angular && g.yaw_rate.abs() <= angular
        }
        ControlIntent::PositionHold(_)
        | ControlIntent::AttitudeThrust(_)
        | ControlIntent::BodyRate(_) => false,
    }
}

/// What an adapter does to a vehicle when its control link is judged lost.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LinkLossPolicy {
    /// Zero all controls immediately.
    Neutralize,
    /// Apply maximum braking.
    Brake,
    /// Hold the last-known controls for a bounded number of ticks, then
    /// neutralize.
    HoldBrief {
        /// Ticks to hold the last-known controls before neutralizing.
        ticks: u32,
    },
    /// Freeze the vehicle in place.
    Pause,
    /// Hand control to an onboard automation system.
    EngageAutomation,
}

/// Why an adapter could not enact a link-loss policy change.
///
/// A failed enactment is a fail-closed fault the driver must count and
/// surface (never a silent no-op): the host has already fenced authority,
/// so an unenacted policy means the vehicle may still be executing its
/// last command with nobody in control.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LinkLossEnactError {
    /// The adapter does not expose the named vehicle.
    #[error("vehicle {vehicle:?} is not exposed by this adapter")]
    UnknownVehicle {
        /// The vehicle the policy change targeted.
        vehicle: pilotage_protocol::VehicleId,
    },
    /// No actuation channel is bound, so the adapter cannot drive the
    /// vehicle to its policy state (e.g. a telemetry-only profile).
    #[error("no actuation channel is bound; the policy cannot be enacted")]
    NoActuationChannel,
    /// The actuation channel refused or dropped the policy command.
    #[error("the actuation channel rejected the policy command: {detail}")]
    ChannelRejected {
        /// Channel-specific failure detail.
        detail: String,
    },
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic)]
mod tests {
    use super::{ApplyOutcome, Disposition, LinkLossPolicy, RejectReason};
    use pilotage_timing::SimTick;

    #[test]
    fn apply_outcome_holds_tick_and_disposition() {
        let outcome = ApplyOutcome::new(SimTick::new(7), Disposition::Accepted);
        assert_eq!(outcome.tick.as_u64(), 7);
        assert_eq!(outcome.disposition, Disposition::Accepted);
        assert!(outcome.action_results.is_empty());
    }

    #[test]
    fn rejected_carries_reason() {
        let disposition = Disposition::Rejected(RejectReason::UnknownScope);
        assert_eq!(
            disposition,
            Disposition::Rejected(RejectReason::UnknownScope)
        );
    }

    #[test]
    fn hold_brief_carries_tick_count() {
        let policy = LinkLossPolicy::HoldBrief { ticks: 5 };
        assert_eq!(policy, LinkLossPolicy::HoldBrief { ticks: 5 });
    }
}
