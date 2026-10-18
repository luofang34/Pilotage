//! The multicopter direct-flight scope at the adapter boundary: what is
//! advertised, which scope admits which family, and per-scope link loss.

use pilotage_adapter_api::{Disposition, LinkLossPolicy, RejectReason, VehicleAdapter};
use pilotage_protocol::{
    BodyRateIntent, ControlIntent, IntentFamily, ScopeId, ScopedControlFrame, VehicleId,
};

use pilotage_mavlink::codec::FcMessage;
use pilotage_mavlink::link::apply_messages_at;

use super::{SOURCE, fake_fc, frame, live_state, uplink_to};
use crate::adapter::{DIRECT_SCOPE, FLIGHT_SCOPE, Px4Adapter};
use crate::fixed_wing::FixedWingUplink;

fn multicopter() -> (Px4Adapter, std::net::UdpSocket) {
    let (fc, addr) = fake_fc();
    let adapter =
        Px4Adapter::from_state(VehicleId::new(1), live_state()).with_uplink(uplink_to(addr));
    (adapter, fc)
}

fn body_rate_frame(scope: &str) -> ScopedControlFrame {
    let mut direct = frame(vec![], vec![]);
    direct.scope = ScopeId::new(scope);
    direct.intent = Some(ControlIntent::BodyRate(BodyRateIntent {
        roll_rate: 0.1,
        pitch_rate: 0.0,
        yaw_rate: 0.0,
        thrust: 0.5,
    }));
    direct
}

#[test]
fn a_multicopter_advertises_body_rate_and_attitude_on_a_grouped_direct_scope() {
    let (adapter, _fc) = multicopter();
    let capabilities = adapter.capabilities();
    let scopes = &capabilities.vehicles[0].scopes;
    let direct = scopes
        .iter()
        .find(|scope| scope.scope.as_str() == DIRECT_SCOPE)
        .expect("direct scope");
    let flight = scopes
        .iter()
        .find(|scope| scope.scope.as_str() == FLIGHT_SCOPE)
        .expect("flight scope");
    let families: Vec<IntentFamily> = direct.intents.iter().map(|intent| intent.family).collect();
    assert_eq!(
        families,
        vec![IntentFamily::BodyRate, IntentFamily::AttitudeThrust]
    );
    assert!(direct.legacy.is_none(), "direct flight is typed only");
    assert_eq!(direct.authority_group, flight.authority_group);
    assert!(direct.authority_group.is_some());
}

#[test]
fn a_fixed_wing_advertises_and_admits_no_direct_flight() {
    let (_fc, addr) = fake_fc();
    let mut adapter = Px4Adapter::from_state(VehicleId::new(1), live_state());
    adapter.fixed_wing = Some(FixedWingUplink::new(addr, 1, 1).expect("uplink"));
    let advertised = adapter.capabilities().vehicles[0]
        .scopes
        .iter()
        .any(|scope| scope.scope.as_str() == DIRECT_SCOPE);
    assert!(!advertised);
    assert_eq!(
        adapter
            .apply_control(&body_rate_frame(DIRECT_SCOPE))
            .disposition,
        Disposition::Rejected(RejectReason::UnknownScope)
    );
}

#[test]
fn each_family_is_admitted_only_on_its_own_scope() {
    let (mut adapter, _fc) = multicopter();
    let rate_on_flight = adapter.apply_control(&body_rate_frame(FLIGHT_SCOPE));
    assert!(
        matches!(
            rate_on_flight.disposition,
            Disposition::Rejected(RejectReason::Other(_))
        ),
        "{rate_on_flight:?}"
    );
    let mut velocity_on_direct = frame(vec![], vec![]);
    velocity_on_direct.scope = ScopeId::new(DIRECT_SCOPE);
    let outcome = adapter.apply_control(&velocity_on_direct);
    assert!(
        matches!(
            outcome.disposition,
            Disposition::Rejected(RejectReason::Other(_))
        ),
        "{outcome:?}"
    );
}

#[test]
fn direct_scope_link_loss_stops_the_stream_and_latches_the_scope() {
    let (mut adapter, _fc) = multicopter();
    adapter.uplink.as_mut().expect("uplink").begin_arm(0.0);
    let scope = ScopeId::new(DIRECT_SCOPE);
    adapter
        .set_link_loss_policy(VehicleId::new(1), &scope, Some(LinkLossPolicy::Neutralize))
        .expect("neutralize");
    assert!(
        !adapter.uplink.as_ref().expect("uplink").streaming(),
        "the stream stops, so PX4's offboard-loss failsafe takes over"
    );
    assert_eq!(
        adapter
            .apply_control(&body_rate_frame(DIRECT_SCOPE))
            .disposition,
        Disposition::Rejected(RejectReason::LinkLossEngaged)
    );
}

/// A scheme switch is release-first, so the host neutralizes the velocity
/// scope before it grants the direct scope: the new holder's first direct
/// frame must resume the stream while PX4 reports itself armed.
#[test]
fn the_next_holder_resumes_the_stream_of_an_armed_vehicle() {
    let state = live_state();
    let (_fc, addr) = fake_fc();
    let mut adapter =
        Px4Adapter::from_state(VehicleId::new(1), state.clone()).with_uplink(uplink_to(addr));
    adapter.uplink.as_mut().expect("uplink").begin_arm(0.0);
    adapter
        .set_link_loss_policy(
            VehicleId::new(1),
            &ScopeId::new(FLIGHT_SCOPE),
            Some(LinkLossPolicy::Neutralize),
        )
        .expect("neutralize");
    let streaming = |adapter: &Px4Adapter| adapter.uplink.as_ref().expect("uplink").streaming();
    assert!(!streaming(&adapter));

    // Not yet reported armed: the stream stays stopped.
    adapter.apply_control(&body_rate_frame(DIRECT_SCOPE));
    assert!(!streaming(&adapter), "a disarmed vehicle is not resumed");

    let heartbeat = FcMessage::Heartbeat {
        armed: true,
        custom_mode: 0,
        autopilot: 12,
    };
    apply_messages_at(
        &state,
        &[(SOURCE, heartbeat)],
        0,
        0,
        std::time::Instant::now(),
    );
    adapter.sample_telemetry();
    adapter.apply_control(&body_rate_frame(DIRECT_SCOPE));
    assert!(streaming(&adapter), "the armed vehicle's stream resumes");
}

#[test]
fn a_disarm_is_never_resumed_by_a_frame() {
    let (mut adapter, _fc) = multicopter();
    let uplink = adapter.uplink.as_mut().expect("uplink");
    uplink.begin_arm(0.0);
    uplink.send_disarm();
    uplink.resume_after_neutralize(true, 0.0, None);
    assert!(!uplink.streaming());
}
