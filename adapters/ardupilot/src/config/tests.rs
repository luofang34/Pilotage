use pilotage_mavlink::codec::{
    EKF_STATUS_REPORT_ID, EXTENDED_SYS_STATE_ID, GIMBAL_DEVICE_ATTITUDE_STATUS_ID,
    GLOBAL_POSITION_INT_ID,
};
use pilotage_mavlink::{AuthorizationSource, CommandTarget, ResetPolicy};

use super::{ArduPilotConfig, ArduPilotProfile};

#[test]
fn the_link_answers_the_fc_and_authorizes_from_the_ekf_report() {
    let config = ArduPilotConfig::new(ArduPilotProfile::Simulation).link_config();
    assert_eq!(config.command_target, CommandTarget::FcSender);
    assert_eq!(
        config.authorization_source,
        AuthorizationSource::ArduPilotEkfStatus
    );
    assert_eq!(config.reset_policy, ResetPolicy::SimulatorHeuristic);
}

#[test]
fn the_ekf_report_is_requested_well_inside_the_authorization_lag() {
    let config = ArduPilotConfig::new(ArduPilotProfile::Simulation).link_config();
    let (_, interval_us) = config
        .stream_interval_requests
        .iter()
        .find(|(id, _)| *id == EKF_STATUS_REPORT_ID)
        .copied()
        .unwrap_or((0, u32::MAX));
    assert!(interval_us.saturating_mul(2) / 1_000 <= config.standard_status_max_lag_ms);
}

#[test]
fn flight_procedures_request_landed_state_and_home_relative_altitude() {
    let config = ArduPilotConfig::new(ArduPilotProfile::Simulation).link_config();
    assert!(
        config
            .stream_interval_requests
            .contains(&(EXTENDED_SYS_STATE_ID, 500_000))
    );
    assert!(
        config
            .stream_interval_requests
            .contains(&(GLOBAL_POSITION_INT_ID, 200_000))
    );
}

#[test]
fn gimbal_feedback_is_requested_only_when_the_payload_is_enabled() {
    let plain = ArduPilotConfig::new(ArduPilotProfile::Simulation);
    assert!(!plain.gimbal);
    let enabled = plain.with_gimbal(true);
    assert!(enabled.gimbal);
    let flight = plain.link_config();
    let gimbal = enabled.link_config();
    assert!(
        !flight
            .stream_interval_requests
            .iter()
            .any(|(id, _)| *id == GIMBAL_DEVICE_ATTITUDE_STATUS_ID)
    );
    assert!(
        gimbal
            .stream_interval_requests
            .contains(&(GIMBAL_DEVICE_ATTITUDE_STATUS_ID, 20_000))
    );
    for request in flight.stream_interval_requests {
        assert!(gimbal.stream_interval_requests.contains(request));
    }
    assert_eq!(
        enabled
            .with_gimbal(false)
            .link_config()
            .stream_interval_requests,
        flight.stream_interval_requests
    );
    assert_eq!(gimbal.authorization_source, flight.authorization_source);
    assert_eq!(gimbal.command_target, flight.command_target);
}
