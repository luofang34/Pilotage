use pilotage_instrument_runtime::Runtime;
use pilotage_protocol::wire;

use crate::{FeedParams, InstrumentFeed};

#[test]
fn reported_angles_cannot_supply_estimator_attitude_or_position() {
    let mut feed = InstrumentFeed::new(&FeedParams {
        vehicle_id: 1,
        sim_accept_unseen: true,
    });
    let stamp = wire::MeasurementStamp {
        role: wire::SourceRole::OperationalEstimate as i32,
        integrity: wire::SourceIntegrity::ChecksummedOnly as i32,
        source_id: 3,
        source_incarnation: vec![7; 16],
        source_epoch: 1,
        sequence: 1,
        acquired_at_ns: 20,
        clock: wire::MeasurementClock::VehicleBoot as i32,
    };
    let telemetry = wire::TelemetrySample {
        vehicle: Some(wire::VehicleId { value: 1 }),
        avionics: Some(wire::AvionicsState {
            reported_euler: Some(wire::ReportedEulerAttitude {
                pitch_rad: 0.3,
                roll_rad: -0.1,
                yaw_rad: 1.5,
                stamp: Some(stamp),
            }),
            quality: 2,
            rate_p_rad_s: f32::NAN,
            rate_q_rad_s: f32::NAN,
            rate_r_rad_s: f32::NAN,
            ..Default::default()
        }),
        ..Default::default()
    };
    assert!(!feed.ingest(&telemetry, 100.0));
    let mut bytes = vec![0; Runtime::state_capacity()];
    let len = feed.state_frame(120.0, &mut bytes).expect("state encodes");
    let report =
        indicate_instrument_state::abi::v8::decode_state(&bytes[..len]).expect("state decodes");
    assert!(report.state.attitude.data.is_none());
    assert!(report.state.kinematics.data.is_none());
    assert!(!report.state.valid.attitude);
    assert!(!report.state.valid.rates);
    assert!(!report.state.valid.position);
}

#[test]
fn canonical_without_rates_keeps_measured_horizon_and_rate_authority_missing() {
    let mut feed = InstrumentFeed::new(&FeedParams {
        vehicle_id: 1,
        sim_accept_unseen: true,
    });
    let stamp = wire::MeasurementStamp {
        role: wire::SourceRole::OperationalEstimate as i32,
        integrity: wire::SourceIntegrity::ChecksummedOnly as i32,
        source_id: 3,
        source_incarnation: vec![7; 16],
        source_epoch: 1,
        sequence: 1,
        acquired_at_ns: 20,
        clock: wire::MeasurementClock::VehicleBoot as i32,
    };
    let telemetry = wire::TelemetrySample {
        vehicle: Some(wire::VehicleId { value: 1 }),
        avionics: Some(wire::AvionicsState {
            quat_w: 0.2_f32.cos(),
            quat_x: 0.2_f32.sin(),
            attitude_stamp: Some(stamp.clone()),
            estimator_status_stamp: Some(stamp),
            valid_flags: 1,
            quality: 0,
            rate_p_rad_s: f32::NAN,
            rate_q_rad_s: f32::NAN,
            rate_r_rad_s: f32::NAN,
            ..Default::default()
        }),
        ..Default::default()
    };
    assert!(feed.ingest(&telemetry, 100.0));
    let mut bytes = vec![0; Runtime::state_capacity()];
    let len = feed.state_frame(120.0, &mut bytes).expect("state encodes");
    let report =
        indicate_instrument_state::abi::v8::decode_state(&bytes[..len]).expect("state decodes");
    let attitude = report.state.attitude.data.expect("measured attitude");
    assert!(attitude.rates_rps.iter().all(|rate| rate.is_nan()));
    assert!(report.state.valid.attitude);
    assert!(!report.state.valid.rates);
    let display = indicate_instrument_state::resolve(
        &report.state,
        &indicate_instrument_state::FreshnessPolicy::default(),
    );
    assert!(display.roll_rad.status.shows_value());
    assert!((display.roll_rad.value - 0.4).abs() < 1e-6);
    assert!(display.pitch_rad.status.shows_value());
    assert_eq!(display.pitch_rad.value, 0.0);
    let mut runtime = Runtime::new();
    runtime.state_mut()[..len].copy_from_slice(&bytes[..len]);
    assert_eq!(
        runtime.render(0).status,
        pilotage_instrument_runtime::RenderStatus::Ok
    );
}
