//! Active control-feel capability notification tests.

use pilotage_adapter_api::{Disposition, VehicleAdapter};

use super::*;

#[test]
fn successful_activation_reports_the_active_descriptor_once() {
    let fc = std::net::UdpSocket::bind("127.0.0.1:0").expect("fake FC");
    let mut adapter = adapter_with_fc(&fc);
    adapter
        .stage_control_feel(candidate("alia250-balanced-notification"))
        .expect("stage candidate");

    assert_eq!(
        adapter.apply_control(&neutral_frame()).disposition,
        Disposition::Accepted
    );
    let active = adapter
        .capabilities()
        .control_feel
        .expect("active descriptor");

    assert_eq!(adapter.take_control_feel_change(), Some(active));
    assert_eq!(adapter.take_control_feel_change(), None);
}

#[test]
fn failed_activation_reports_no_descriptor() {
    let fc = std::net::UdpSocket::bind("127.0.0.1:0").expect("fake FC");
    let mut adapter = airborne_adapter_with_fc(&fc);
    adapter
        .stage_control_feel(candidate("alia250-balanced-failed-notification"))
        .expect("stage candidate");
    adapter
        .uplink_mut()
        .expect("uplink")
        .set_target("[::1]:9".parse().expect("IPv6 target"));

    assert!(matches!(
        adapter.apply_control(&neutral_frame()).disposition,
        Disposition::Rejected(_)
    ));
    assert_eq!(adapter.take_control_feel_change(), None);
}

#[test]
fn rollback_reports_the_complete_prior_descriptor_once() {
    let fc = std::net::UdpSocket::bind("127.0.0.1:0").expect("fake FC");
    let mut adapter = adapter_with_fc(&fc);
    let prior = adapter
        .capabilities()
        .control_feel
        .expect("prior descriptor");
    adapter
        .stage_control_feel(candidate("alia250-balanced-before-rollback"))
        .expect("stage candidate");
    assert_eq!(
        adapter.apply_control(&neutral_frame()).disposition,
        Disposition::Accepted
    );
    assert!(adapter.take_control_feel_change().is_some());

    assert!(adapter.stage_control_feel_rollback());
    assert_eq!(
        adapter.apply_control(&neutral_frame()).disposition,
        Disposition::Accepted
    );

    assert_eq!(adapter.take_control_feel_change(), Some(prior));
    assert_eq!(adapter.take_control_feel_change(), None);
}
