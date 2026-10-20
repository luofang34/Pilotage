//! A fixed-wing PX4 advertises the envelope its stick mapping flies and no
//! lateral route, and its flight frames reach the fixed-wing uplink.

use pilotage_adapter_api::{Disposition, LegacyCommandMap, RejectReason, VehicleAdapter};
use pilotage_protocol::{ControlAction, VehicleId};

use super::{fake_fc, frame, live_state};
use crate::adapter::{FLIGHT_SCOPE, Px4Adapter};
use crate::fixed_wing::FixedWingUplink;
use crate::fixed_wing::sticks::{MAX_AIRSPEED_CHANGE_MPS, MAX_VERTICAL_MPS, max_turn_rate_rps};

fn fixed_wing_adapter() -> Px4Adapter {
    let (_fc, addr) = fake_fc();
    let mut adapter = Px4Adapter::from_state(VehicleId::new(1), live_state());
    adapter.fixed_wing = Some(
        FixedWingUplink::new(crate::config::Px4Profile::Simulation, addr, 1, 1).expect("uplink"),
    );
    adapter
}

#[test]
fn a_fixed_wing_advertises_its_own_envelope_and_no_lateral_route() {
    let capabilities = fixed_wing_adapter().capabilities();
    let flight = capabilities.vehicles[0]
        .scopes
        .iter()
        .find(|scope| scope.scope.as_str() == FLIGHT_SCOPE)
        .expect("flight scope");
    let intent = &flight.intents[0];
    assert_eq!(intent.max_linear, MAX_AIRSPEED_CHANGE_MPS);
    assert_eq!(intent.max_vertical, MAX_VERTICAL_MPS);
    assert_eq!(intent.max_angular, max_turn_rate_rps());
    let Some(LegacyCommandMap::Velocity { vy, vx, .. }) = &flight.legacy else {
        panic!("velocity legacy map expected, got {:?}", flight.legacy);
    };
    assert!(vy.is_none(), "a fixed wing routes no axis to vy");
    assert!(vx.is_some());
    assert_eq!(
        capabilities.vehicles[0].link_loss_actions,
        vec![pilotage_adapter_api::LinkLossPolicy::Neutralize]
    );
}

#[test]
fn fixed_wing_flight_frames_reach_the_fixed_wing_uplink() {
    let mut adapter = fixed_wing_adapter();
    // No arm sequence yet: the stick intent is not enacted.
    let outcome = adapter.apply_control(&frame(vec![], vec![]));
    assert_eq!(
        outcome.disposition,
        Disposition::Rejected(RejectReason::UplinkIdle)
    );
    // ARM waits for PX4 to confirm the pinned parameters.
    let mut press = frame(vec![], vec![]);
    press.actions = vec![ControlAction::Arm];
    let outcome = adapter.apply_control(&press);
    assert!(!outcome.action_results[0].accepted);
    assert!(
        outcome.action_results[0]
            .detail
            .contains("waiting for PX4 to confirm COM_RC_IN_MODE = 1"),
        "{:?}",
        outcome.action_results[0]
    );
}
