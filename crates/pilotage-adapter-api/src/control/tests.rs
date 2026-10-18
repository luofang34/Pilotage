//! Apply outcomes, link-loss policies, and the neutral-activation gate.

use pilotage_protocol::{BodyRateIntent, ControlIntent};

use super::{
    ApplyOutcome, Disposition, LinkLossPolicy, RejectReason, intent_satisfies_neutral_activation,
};
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

fn body_rate_capability() -> crate::IntentCapability {
    crate::IntentCapability {
        family: pilotage_protocol::IntentFamily::BodyRate,
        frames: vec![],
        max_linear: 0.0,
        max_vertical: 0.0,
        max_angular: 3.5,
        max_yaw_rate: 0.0,
    }
}

fn body_rate(roll_rate: f32, pitch_rate: f32, yaw_rate: f32, thrust: f32) -> ControlIntent {
    ControlIntent::BodyRate(BodyRateIntent {
        roll_rate,
        pitch_rate,
        yaw_rate,
        thrust,
    })
}

/// A 5% deadband: 0.175 rad/s of rate, 0.025 of thrust around 0.5.
const DEADBAND_MILLI: u32 = 50;

#[test]
fn zero_body_rates_at_centre_thrust_are_neutral() {
    let capability = body_rate_capability();
    for intent in [
        body_rate(0.0, 0.0, 0.0, 0.5),
        body_rate(0.17, -0.17, 0.17, 0.52),
        body_rate(-0.1, 0.1, -0.1, 0.48),
    ] {
        assert!(
            intent_satisfies_neutral_activation(&intent, &capability, DEADBAND_MILLI),
            "{intent:?} is neutral"
        );
    }
}

#[test]
fn a_deflected_rate_or_off_centre_thrust_is_not_neutral() {
    let capability = body_rate_capability();
    for intent in [
        body_rate(0.2, 0.0, 0.0, 0.5),
        body_rate(0.0, -0.2, 0.0, 0.5),
        body_rate(0.0, 0.0, 1.0, 0.5),
        body_rate(0.0, 0.0, 0.0, 0.57),
        body_rate(0.0, 0.0, 0.0, 0.0),
        body_rate(f32::NAN, 0.0, 0.0, 0.5),
    ] {
        assert!(
            !intent_satisfies_neutral_activation(&intent, &capability, DEADBAND_MILLI),
            "{intent:?} is not neutral"
        );
    }
}
