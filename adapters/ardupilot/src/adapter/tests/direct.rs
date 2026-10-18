//! The direct scope through the adapter: it shares the velocity scope's
//! authority, refuses frames until the FC's thrust scale is confirmed,
//! streams body rates with hover-centred thrust, and neutralizes to a
//! velocity hold on link loss.

use pilotage_adapter_api::{Disposition, LinkLossPolicy, RejectReason, VehicleAdapter};
use pilotage_mavlink::codec::{AttitudeTargetKind, FcMessage, ParamValue, param_id};
use pilotage_mavlink::{OffboardSetpoint, ParamRequest};
use pilotage_protocol::{BodyRateIntent, ControlIntent, ScopeId, VehicleId};

use super::{Harness, body_velocity, scoped, velocity};
use crate::adapter::{DIRECT_SCOPE, FLIGHT_SCOPE};

fn rates(roll_rate: f32, thrust: f32) -> Option<ControlIntent> {
    Some(ControlIntent::BodyRate(BodyRateIntent {
        roll_rate,
        pitch_rate: 0.0,
        yaw_rate: 0.0,
        thrust,
    }))
}

/// Reports the two parameters the way ArduCopter does once bit 3 is set.
fn confirm_parameters(harness: &mut Harness) {
    harness.feed(&[
        FcMessage::ParamValue {
            param_id: param_id("GUID_OPTIONS"),
            value: ParamValue::Int32(8.0_f32.to_bits() as i32),
        },
        FcMessage::ParamValue {
            param_id: param_id("MOT_THST_HOVER"),
            value: ParamValue::Real32(0.39),
        },
    ]);
    harness.adapter.sample_telemetry();
}

#[test]
fn both_flight_scopes_share_one_authority_group() {
    let harness = Harness::new();
    let capabilities = harness.adapter.capabilities();
    let scopes = &capabilities.vehicles[0].scopes;
    let group = |name: &str| {
        scopes
            .iter()
            .find(|scope| scope.scope.as_str() == name)
            .and_then(|scope| scope.authority_group.clone())
    };
    assert_eq!(group(FLIGHT_SCOPE), Some(FLIGHT_SCOPE.to_owned()));
    assert_eq!(group(DIRECT_SCOPE), Some(FLIGHT_SCOPE.to_owned()));
}

#[test]
fn direct_frames_wait_for_the_thrust_scale_and_then_stream_rates() {
    let mut harness = Harness::new();
    harness.estimate(-3.0, true);
    let early = harness
        .adapter
        .apply_control(&scoped(DIRECT_SCOPE, rates(0.5, 0.5), vec![]));
    assert!(matches!(
        early.disposition,
        Disposition::Rejected(RejectReason::Other(_))
    ));
    let reads: Vec<_> = std::iter::from_fn(|| harness.params.try_recv().ok()).collect();
    assert!(
        reads
            .iter()
            .any(|p| p.request == ParamRequest::Read("MOT_THST_HOVER"))
    );

    confirm_parameters(&mut harness);
    let streamed = harness
        .adapter
        .apply_control(&scoped(DIRECT_SCOPE, rates(0.5, 0.5), vec![]));
    assert_eq!(streamed.disposition, Disposition::Accepted);
    let Some(OffboardSetpoint::AttitudeThrust(demand)) = *harness.velocity.borrow_and_update()
    else {
        panic!("an attitude-thrust setpoint");
    };
    assert_eq!(demand.kind, AttitudeTargetKind::BodyRates([0.5, 0.0, 0.0]));
    assert_eq!(
        demand.thrust, 0.39,
        "canonical 0.5 is the FC's hover thrust"
    );
}

#[test]
fn a_velocity_intent_on_the_direct_scope_is_refused() {
    let mut harness = Harness::new();
    harness.estimate(-3.0, true);
    confirm_parameters(&mut harness);
    let outcome = harness.adapter.apply_control(&scoped(
        DIRECT_SCOPE,
        velocity(1.0, 0.0, 0.0).map(ControlIntent::Velocity),
        vec![],
    ));
    assert!(matches!(
        outcome.disposition,
        Disposition::Rejected(RejectReason::Other(_))
    ));
}

#[test]
fn direct_link_loss_brakes_to_a_velocity_hold() {
    let mut harness = Harness::new();
    harness.estimate(-3.0, true);
    confirm_parameters(&mut harness);
    harness
        .adapter
        .apply_control(&scoped(DIRECT_SCOPE, rates(1.0, 0.6), vec![]));
    harness.velocity.borrow_and_update();
    harness
        .adapter
        .set_link_loss_policy(
            VehicleId::new(1),
            &ScopeId::new(DIRECT_SCOPE),
            Some(LinkLossPolicy::Neutralize),
        )
        .expect("engaged");
    let hold = body_velocity(*harness.velocity.borrow_and_update()).expect("a velocity hold");
    assert_eq!(hold.vel_frd_mps, [0.0; 3]);
}
