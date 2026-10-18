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

fn armed_heartbeat(state: &std::sync::Arc<std::sync::Mutex<pilotage_mavlink::LinkState>>) {
    let heartbeat = FcMessage::Heartbeat {
        armed: true,
        custom_mode: 0,
        autopilot: 12,
    };
    apply_messages_at(
        state,
        &[(SOURCE, heartbeat)],
        0,
        0,
        std::time::Instant::now(),
    );
}

fn at_generation(mut frame: ScopedControlFrame, generation: u64) -> ScopedControlFrame {
    frame.generation = pilotage_protocol::Generation::new(generation);
    frame
}

/// A flying multicopter whose velocity lease (generation 1) the host has
/// just neutralized, as it does on a voluntary scope transfer.
fn neutralized_in_flight(armed: bool) -> (Px4Adapter, std::net::UdpSocket) {
    let state = live_state();
    let (fc, addr) = fake_fc();
    let mut adapter =
        Px4Adapter::from_state(VehicleId::new(1), state.clone()).with_uplink(uplink_to(addr));
    if armed {
        armed_heartbeat(&state);
    }
    adapter.sample_telemetry();
    adapter.uplink.as_mut().expect("uplink").begin_arm(0.0);
    adapter.apply_control(&at_generation(frame(vec![], vec![]), 1));
    adapter
        .set_link_loss_policy(
            VehicleId::new(1),
            &ScopeId::new(FLIGHT_SCOPE),
            Some(LinkLossPolicy::Neutralize),
        )
        .expect("neutralize");
    assert!(!adapter.uplink.as_ref().expect("uplink").streaming());
    // While the group latch holds, a direct frame cannot fly around it.
    assert_eq!(
        adapter
            .apply_control(&at_generation(body_rate_frame(DIRECT_SCOPE), 2))
            .disposition,
        Disposition::Rejected(RejectReason::LinkLossEngaged)
    );
    // The host clears the latch on the new holder's neutral demonstration,
    // before it applies that frame.
    adapter
        .set_link_loss_policy(VehicleId::new(1), &ScopeId::new(FLIGHT_SCOPE), None)
        .expect("clear");
    (adapter, fc)
}

fn streaming(adapter: &Px4Adapter) -> bool {
    adapter.uplink.as_ref().expect("uplink").streaming()
}

#[test]
fn a_newer_generation_direct_frame_resumes_the_stream_then_asks_for_offboard() {
    let (mut adapter, _fc) = neutralized_in_flight(true);
    let uplink = adapter.uplink.as_mut().expect("uplink");
    uplink.use_manual_clock();
    adapter.apply_control(&at_generation(body_rate_frame(DIRECT_SCOPE), 2));
    assert!(streaming(&adapter), "the newer holder's stream resumes");
    let uplink = adapter.uplink.as_mut().expect("uplink");
    assert!(uplink.offboard_request_pending(), "setpoints flow first");
    uplink.advance_clock(std::time::Duration::from_millis(300));
    uplink.maintain();
    assert!(
        !uplink.offboard_request_pending(),
        "OFFBOARD follows the warmup"
    );
}

#[test]
fn a_same_generation_frame_does_not_revive_the_stream() {
    let (mut adapter, _fc) = neutralized_in_flight(true);
    adapter.apply_control(&at_generation(body_rate_frame(DIRECT_SCOPE), 1));
    adapter.apply_control(&at_generation(frame(vec![], vec![]), 1));
    assert!(!streaming(&adapter));
}

#[test]
fn a_neutralized_disarmed_vehicle_stays_idle() {
    let (mut adapter, _fc) = neutralized_in_flight(false);
    adapter.apply_control(&at_generation(body_rate_frame(DIRECT_SCOPE), 2));
    assert!(!streaming(&adapter));
}

#[test]
fn a_disarm_is_never_resumed_by_a_frame() {
    let (mut adapter, _fc) = multicopter();
    let uplink = adapter.uplink.as_mut().expect("uplink");
    uplink.begin_arm(0.0);
    uplink.send_disarm();
    uplink.admit_flight_frame(pilotage_protocol::Generation::new(9), true, 0.0, None);
    assert!(!uplink.streaming());
}

/// The link-loss enactment leaves the vehicle to PX4's offboard-loss
/// failsafe, so arming waits until PX4 confirms that failsafe is Hold.
#[test]
fn arming_waits_for_the_pinned_offboard_loss_hold() {
    let (fc, addr) = fake_fc();
    let mut adapter = Px4Adapter::from_state(VehicleId::new(1), live_state())
        .with_uplink(crate::uplink::Px4Uplink::new(addr).expect("uplink"));
    let mut arm = frame(vec![], vec![]);
    arm.actions = vec![pilotage_protocol::ControlAction::Arm];
    let refused = adapter.apply_control(&arm);
    assert!(
        refused
            .action_results
            .first()
            .is_some_and(|result| !result.accepted),
        "{refused:?}"
    );
    assert!(!streaming(&adapter));

    // The uplink writes COM_OBL_RC_ACT = 5 and arms once PX4 echoes it.
    adapter.uplink.as_mut().expect("uplink").maintain();
    let mut buf = [0u8; 280];
    let mut wrote_hold = false;
    fc.set_read_timeout(Some(std::time::Duration::from_secs(1)))
        .expect("timeout");
    while let Ok(len) = fc.recv(&mut buf) {
        // MAVLink 2: message id 23 (PARAM_SET) at bytes 7..10.
        let is_param_set = len > 10 && buf[7..10] == [23, 0, 0];
        if is_param_set && buf[..len].windows(14).any(|w| w == b"COM_OBL_RC_ACT") {
            wrote_hold = true;
            break;
        }
    }
    assert!(wrote_hold, "a PARAM_SET went to PX4");
    adapter.uplink.as_mut().expect("uplink").confirm_failsafe();
    let accepted = adapter.apply_control(&arm);
    assert!(
        accepted
            .action_results
            .first()
            .is_some_and(|result| result.accepted),
        "{accepted:?}"
    );
    assert!(streaming(&adapter));
}
