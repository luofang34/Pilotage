use pilotage_mavlink::codec::EKF_STATUS_REPORT_ID;
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
