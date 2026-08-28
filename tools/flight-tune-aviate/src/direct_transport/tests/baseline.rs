//! Freezing the direct baseline for one run.

use super::super::{DirectSetpoint, DirectTransportError, EffectiveSetpointReport};
use super::sender::RecordingSender;
use super::{HOVER_TRIM, SAMPLE_PERIOD_NS, authorize, baseline_request};

fn active_direct_setpoint() -> DirectSetpoint {
    DirectSetpoint {
        roll_rad: 0.14,
        pitch_rad: -0.09,
        yaw_rad: 2.1,
        collective_force: 0.55,
    }
}

fn active_report() -> EffectiveSetpointReport {
    EffectiveSetpointReport {
        setpoint: active_direct_setpoint(),
        sample_sequence: 0,
        sample_time_ns: 0,
        estimate_time_ns: 0,
        simulator_truth_time_ns: 0,
    }
}

#[test]
fn a_direct_step_starts_from_the_recorded_effective_setpoint() {
    let mut sender = RecordingSender::new().reporting(active_report());
    let mut transport = authorize(&sender);

    let baseline = transport
        .freeze_baseline_blocking(&mut sender, &baseline_request())
        .expect("frozen baseline");

    let active = active_direct_setpoint();
    assert_eq!(baseline.setpoint().roll_rad, active.roll_rad);
    assert_eq!(baseline.setpoint().pitch_rad, active.pitch_rad);
    assert_eq!(baseline.setpoint().yaw_rad, active.yaw_rad);
    assert_eq!(
        baseline.setpoint().collective_force,
        HOVER_TRIM,
        "neutral collective force is the identified hover trim"
    );
    assert_eq!(baseline.hover_trim(), HOVER_TRIM);
}

#[test]
fn a_baseline_uses_the_measured_attitude_when_no_direct_setpoint_is_active() {
    let mut sender = RecordingSender::new();
    let mut transport = authorize(&sender);
    let request = baseline_request();

    let baseline = transport
        .freeze_baseline_blocking(&mut sender, &request)
        .expect("frozen baseline");

    assert_eq!(baseline.setpoint().roll_rad, request.measured_roll_rad);
    assert_eq!(baseline.setpoint().pitch_rad, request.measured_pitch_rad);
    assert_eq!(baseline.setpoint().yaw_rad, request.measured_yaw_rad);
    assert_eq!(baseline.setpoint().collective_force, HOVER_TRIM);
}

#[test]
fn the_baseline_block_sends_the_exact_baseline_until_the_vehicle_is_stable() {
    let mut sender = RecordingSender::new().unstable_for(2);
    let mut transport = authorize(&sender);

    let baseline = transport
        .freeze_baseline_blocking(&mut sender, &baseline_request())
        .expect("frozen baseline");

    assert_eq!(baseline.commands(), 3, "the block is continuous");
    assert_eq!(sender.transmitted().len(), 3);
    for sent in sender.transmitted() {
        assert_eq!(
            *sent,
            baseline.setpoint(),
            "every command in the block is the same exact baseline"
        );
    }
    assert_eq!(baseline.frozen_at_ns(), 3 * SAMPLE_PERIOD_NS);
}

#[test]
fn a_baseline_that_never_settles_fails_closed() {
    let mut sender = RecordingSender::new().unstable_for(100);
    let mut transport = authorize(&sender);
    let mut request = baseline_request();
    request.max_commands = 4;

    let result = transport.freeze_baseline_blocking(&mut sender, &request);

    assert!(matches!(
        result,
        Err(DirectTransportError::BaselineNotSettled { commands: 4 })
    ));
    assert!(transport.baseline().is_none());
}

#[test]
fn a_baseline_whose_readback_never_matches_fails_closed() {
    let substitute = DirectSetpoint {
        roll_rad: 9.0,
        pitch_rad: 9.0,
        yaw_rad: 9.0,
        collective_force: 0.9,
    };
    let mut sender = RecordingSender::new().substituting(substitute);
    let mut transport = authorize(&sender);
    let mut request = baseline_request();
    request.max_commands = 3;

    let result = transport.freeze_baseline_blocking(&mut sender, &request);

    assert!(matches!(
        result,
        Err(DirectTransportError::BaselineNotSettled { commands: 3 })
    ));
}

#[test]
fn a_frozen_baseline_cannot_be_frozen_again_for_the_run() {
    let mut sender = RecordingSender::new();
    let mut transport = authorize(&sender);
    transport
        .freeze_baseline_blocking(&mut sender, &baseline_request())
        .expect("frozen baseline");

    let result = transport.freeze_baseline_blocking(&mut sender, &baseline_request());

    assert!(matches!(result, Err(DirectTransportError::BaselineFrozen)));
}

#[test]
fn an_incomplete_baseline_request_is_refused() {
    let mut sender = RecordingSender::new();
    let mut transport = authorize(&sender);
    let mut request = baseline_request();
    request.hover_trim = f64::NAN;

    let result = transport.freeze_baseline_blocking(&mut sender, &request);

    assert!(matches!(
        result,
        Err(DirectTransportError::InvalidValue {
            field: "hover trim"
        })
    ));
    assert!(sender.transmitted().is_empty());
}
