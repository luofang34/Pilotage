//! Pins for the packed wire layouts. The runtime returns typed outcomes;
//! this adapter's packing is the ABI the script consumes, so the bit
//! layout is pinned here, next to the packing.

#![allow(clippy::expect_used, clippy::panic)]

use indicate_instrument_state::abi::v7::{VERSION, encode_state};
use indicate_instrument_state::{AircraftState, Attitude, Quat, Stamped};
use pilotage_instrument_runtime::RenderStatus;

use crate::{InstrumentRuntime, abi_version};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PackedResult {
    status: u32,
    scene_len: u32,
    generation: u32,
}

fn unpack(result: u64) -> PackedResult {
    PackedResult {
        status: (result & 0xff) as u32,
        scene_len: ((result >> 8) & 0x00ff_ffff) as u32,
        generation: (result >> 32) as u32,
    }
}

fn write_state(resource: &mut InstrumentRuntime, state: &AircraftState) {
    let runtime = resource.runtime.as_mut().expect("initialized");
    runtime.state_mut().fill(0);
    encode_state(state, runtime.state_mut()).expect("encodes");
}

fn attitude_state() -> AircraftState {
    AircraftState {
        attitude: Stamped {
            data: Some(Attitude {
                quat: Quat::IDENTITY,
                rates_rps: [0.0; 3],
            }),
            age_ms: Some(10.0),
        },
        valid: indicate_instrument_state::ValidFlags {
            attitude: true,
            rates: true,
            ..Default::default()
        },
        ..AircraftState::default()
    }
}

#[test]
fn render_result_packing_matches_the_documented_layout() {
    assert_eq!(abi_version(), u32::from(VERSION));
    let mut resource = InstrumentRuntime::new();
    assert_eq!(resource.state_ptr(), 0);
    assert_eq!(resource.scene_ptr(), 0);
    assert_eq!(
        unpack(resource.render_result(0)),
        PackedResult {
            status: RenderStatus::NotInitialized as u32,
            scene_len: 0,
            generation: 0,
        }
    );
    assert_eq!(
        resource.set_v_speeds(40.0, 48.0, 85.0, 129.0, 163.0),
        RenderStatus::NotInitialized as u32
    );

    assert_eq!(resource.init(), 1);
    assert_ne!(resource.state_ptr(), 0);
    assert_ne!(resource.scene_ptr(), 0);
    write_state(&mut resource, &attitude_state());
    let raw = resource.render_result(0);
    let result = unpack(raw);
    assert_eq!(result.status, RenderStatus::Ok as u32);
    assert!(result.scene_len > 1, "panel rendered no scene");
    assert_eq!(result.generation, 1);
    assert_eq!(
        raw,
        (u64::from(result.generation) << 32) | (u64::from(result.scene_len) << 8),
        "packed ABI layout"
    );

    let runtime = resource.runtime.as_mut().expect("initialized");
    runtime.state_mut()[0..4].copy_from_slice(&99u32.to_le_bytes());
    let raw = resource.render_result(0);
    let failure = unpack(raw);
    assert_eq!(failure.status, RenderStatus::StateBadVersion as u32);
    assert_eq!(failure.scene_len, 0);
    assert_eq!(failure.generation, 1, "failure never advances generation");
    assert_eq!(
        raw,
        (u64::from(failure.generation) << 32) | u64::from(failure.status)
    );
}

#[test]
fn step_alerts_packing_matches_the_documented_layout() {
    let mut resource = InstrumentRuntime::new();
    assert_eq!(
        resource.step_alerts(1_000, 1),
        RenderStatus::NotInitialized as u64
    );

    // A failed altitude source forces an alert transition, so the
    // manager generation provably moves into bits 32..63.
    let mut failed_alt = attitude_state();
    failed_alt.kinematics = Stamped {
        data: Some(indicate_instrument_state::Kinematics {
            pos_ned_m: [0.0, 0.0, -300.0],
            vel_ned_mps: [0.0; 3],
        }),
        age_ms: Some(10.0),
    };
    failed_alt.valid.position = false;
    failed_alt.valid.velocity_horizontal = false;
    failed_alt.valid.velocity_vertical = false;

    resource.init();
    write_state(&mut resource, &failed_alt);
    let summary = resource.step_alerts(1_000, 1);
    assert_eq!(summary & 0xff, RenderStatus::Ok as u64);
    assert!(
        (summary >> 8) & 0xff >= 1,
        "active-alert count in bits 8..15"
    );
    assert_eq!((summary >> 16) & 1, 0, "healthy path is not faulted");
    assert_eq!((summary >> 17) & 1, 0, "no overflow");
    assert_eq!((summary >> 18) & 0x3fff, 0, "bits 18..31 stay zero");
    assert_eq!((summary >> 32), 1, "manager generation in bits 32..63");

    let faulted = resource.step_alerts(2_000, 0);
    assert_eq!((faulted >> 16) & 1, 1, "monitor fault sets bit 16");
    assert_eq!((faulted >> 32), 1, "no new transition, generation holds");
}

#[test]
fn resources_are_independent_and_reinitialization_resets_one() {
    let mut first = InstrumentRuntime::new();
    let mut second = InstrumentRuntime::new();
    first.init();
    second.init();
    write_state(&mut first, &attitude_state());
    assert_eq!(unpack(first.render_result(0)).generation, 1);
    assert_eq!(unpack(second.render_result(0)).generation, 0);

    first.init();
    let reinitialized = unpack(first.render_result(0));
    assert_eq!(reinitialized.status, RenderStatus::StateBadVersion as u32);
    assert_eq!(reinitialized.generation, 0);
}
