#![allow(clippy::expect_used, clippy::panic)]

//! End-to-end feed tests: a wire telemetry sample becomes a state frame
//! the shared instrument runtime accepts and renders.

use pilotage_instrument_runtime::Runtime;
use pilotage_protocol::wire;

use crate::{FeedParams, InstrumentFeed};

/// A level-flight wire sample with full estimator authorization, stamped
/// the way the simulation adapter stamps: one source, one incarnation,
/// both groups on the same acquisition instant.
fn wire_sample(sequence: u32, yaw_rad: f64) -> wire::TelemetrySample {
    let stamp = |seq: u32| {
        Some(wire::MeasurementStamp {
            source_id: 11,
            source_epoch: 1,
            sequence: seq,
            acquired_at_ns: u64::from(seq) * 10_000_000,
            clock: 1,
            source_incarnation: vec![7_u8; 16],
            role: 1,
            integrity: 1,
        })
    };
    let half = yaw_rad / 2.0;
    #[allow(clippy::cast_possible_truncation, deprecated)]
    let avionics = wire::AvionicsState {
            baro_alt_m: 0.0,
            baro_stamp: None,
        quat_w: half.cos() as f32,
        quat_x: 0.0,
        quat_y: 0.0,
        quat_z: half.sin() as f32,
        rate_p_rad_s: 0.0,
        rate_q_rad_s: 0.0,
        rate_r_rad_s: 0.0,
        pos_n_m: 10.0,
        pos_e_m: 20.0,
        pos_d_m: -30.0,
        vel_n_mps: 5.0,
        vel_e_mps: 0.0,
        vel_d_mps: -1.0,
        valid_flags: 0b1111,
        quality: 0,
        arm_state: 0,
        attitude_stamp: stamp(sequence),
        kinematics_stamp: stamp(sequence),
        estimator_status_stamp: stamp(sequence),
    };
    wire::TelemetrySample {
        vehicle: Some(wire::VehicleId { value: 1 }),
        avionics: Some(avionics),
        ..Default::default()
    }
}

#[test]
fn a_wire_sample_becomes_a_state_frame_the_runtime_renders() {
    let mut feed = InstrumentFeed::new(&FeedParams {
        vehicle_id: 1,
        sim_accept_unseen: true,
    });
    assert!(
        feed.ingest(&wire_sample(1, 0.5), 100.0),
        "an authorized publication must be admitted"
    );

    let mut runtime = Runtime::new();
    let capacity = Runtime::state_capacity();
    let mut buf = vec![0_u8; capacity];
    let len = feed
        .state_frame(116.0, &mut buf)
        .expect("the state buffer holds the frame");
    assert!(len > 2, "an admitted feed emits at least one group");
    runtime.state_mut()[..len].copy_from_slice(&buf[..len]);

    let outcome = runtime.render(0);
    assert_eq!(
        outcome.status,
        pilotage_instrument_runtime::RenderStatus::Ok,
        "the first panel renders from the assembled frame"
    );
    assert!(outcome.scene_len > 0);
}

#[test]
fn a_sample_for_another_vehicle_is_refused_by_the_shared_ingress() {
    let mut feed = InstrumentFeed::new(&FeedParams {
        vehicle_id: 2,
        sim_accept_unseen: true,
    });
    assert!(
        !feed.ingest(&wire_sample(1, 0.0), 100.0),
        "vehicle 1's publication must not feed vehicle 2's instruments"
    );
}

#[test]
fn absence_of_avionics_feeds_nothing() {
    let mut feed = InstrumentFeed::new(&FeedParams {
        vehicle_id: 1,
        sim_accept_unseen: true,
    });
    let empty = wire::TelemetrySample {
        vehicle: Some(wire::VehicleId { value: 1 }),
        ..Default::default()
    };
    assert!(!feed.ingest(&empty, 50.0));
    let mut buf = vec![0_u8; Runtime::state_capacity()];
    let len = feed
        .state_frame(60.0, &mut buf)
        .expect("an empty frame still encodes");
    // Version byte, count byte, and whatever the always-present groups
    // (quality, validity, snapshot meta) occupy — but no attitude.
    let report = indicate_instrument_state::abi::v7::decode_state(&buf[..len])
        .expect("the encoded frame decodes");
    assert!(
        report.state.attitude.data.is_none(),
        "no attitude group may appear before one is admitted"
    );
}

#[test]
fn stamp_skew_the_browser_accepts_does_not_flag_coherence() {
    // The browser admits up to 300 ms of attitude/kinematics acquisition
    // skew. A tighter budget here made the native panels flag coherence
    // the web accepted — and flap as real skew wandered across it.
    let mut feed = InstrumentFeed::new(&FeedParams {
        vehicle_id: 1,
        sim_accept_unseen: true,
    });
    let mut sample = wire_sample(1, 0.0);
    if let Some(avionics) = sample.avionics.as_mut()
        && let Some(stamp) = avionics.kinematics_stamp.as_mut()
    {
        stamp.acquired_at_ns += 200_000_000;
    }
    assert!(feed.ingest(&sample, 100.0));

    let mut buf = vec![0_u8; Runtime::state_capacity()];
    let len = feed
        .state_frame(120.0, &mut buf)
        .expect("the frame encodes");
    let report =
        indicate_instrument_state::abi::v7::decode_state(&buf[..len]).expect("the frame decodes");
    assert_eq!(
        report.state.snapshot.coherence,
        indicate_instrument_state::SnapshotCoherence::Coherent,
        "200 ms of skew is inside the shared budget"
    );
}

#[test]
fn the_heading_bug_carries_a_declared_reference() {
    // A bug against an Unknown north fail-closes invisible; the feed
    // declares the simulation north the way the browser does.
    let mut feed = InstrumentFeed::new(&FeedParams {
        vehicle_id: 1,
        sim_accept_unseen: true,
    });
    feed.ingest(&wire_sample(1, 0.0), 100.0);
    let mut buf = vec![0_u8; Runtime::state_capacity()];
    let len = feed.state_frame(120.0, &mut buf).expect("frame encodes");
    let report =
        indicate_instrument_state::abi::v7::decode_state(&buf[..len]).expect("frame decodes");
    assert_eq!(
        report.state.selections.heading_bug_reference,
        indicate_instrument_state::HeadingReference::SimLocalTrue,
        "the bug must state which north it is measured from"
    );
}
