#![allow(clippy::expect_used, clippy::panic)]

use pilotage_instrument_panels::PfdConfig;
use pilotage_instrument_scene::SceneCmds;
use pilotage_instrument_state::abi::{STATE_ABI_SIZE, STATE_ABI_VERSION, encode_state};
use pilotage_instrument_state::{AircraftState, Attitude, Quat, Stamped};

use super::{InstrumentRuntime, RenderStatus, abi_version};
use crate::exports::{
    RenderAttempt, Runtime, SCENE_CAPACITY, render_into, validate_and_commit_scene,
};

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

fn write_state(runtime: &mut Runtime, state: &AircraftState) {
    let mut block = vec![0u8; STATE_ABI_SIZE];
    encode_state(state, &mut block).expect("encodes");
    runtime.state.copy_from_slice(&block);
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
        ..AircraftState::default()
    }
}

fn encoded_state_block(state: &AircraftState) -> Vec<u8> {
    let mut block = vec![0u8; STATE_ABI_SIZE];
    encode_state(state, &mut block).expect("encodes");
    block
}

fn assert_attempt(attempt: RenderAttempt, status: RenderStatus, scene_len: usize, generation: u32) {
    assert_eq!(attempt.status, status);
    assert_eq!(attempt.scene_len, scene_len);
    assert_eq!(attempt.generation, generation);
}

#[test]
fn exported_surface_packs_one_atomic_render_result() {
    assert_eq!(abi_version(), STATE_ABI_VERSION);
    let mut resource = InstrumentRuntime::new();
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
    assert_eq!(resource.state_ptr(), 0);
    assert_eq!(resource.scene_ptr(), 0);
    assert_eq!(resource.state_len() as usize, STATE_ABI_SIZE);

    assert_eq!(resource.init(), 1);
    assert_ne!(resource.state_ptr(), 0);
    assert_ne!(resource.scene_ptr(), 0);
    assert_eq!(
        resource.set_v_speeds(40.0, 48.0, 85.0, 129.0, 163.0),
        RenderStatus::Ok as u32
    );

    let bad_version = unpack(resource.render_result(0));
    assert_eq!(bad_version.status, RenderStatus::StateBadVersion as u32);
    assert_eq!(bad_version.scene_len, 0);
    assert_eq!(bad_version.generation, 0);

    write_state(
        resource.runtime.as_mut().expect("initialized"),
        &attitude_state(),
    );
    for panel in [0u32, 1] {
        let raw = resource.render_result(panel);
        let result = unpack(raw);
        assert_eq!(result.status, RenderStatus::Ok as u32);
        assert!(result.scene_len > 1, "panel {panel} rendered no scene");
        assert_eq!(result.generation, 1);
        assert_eq!(
            raw,
            (u64::from(result.generation) << 32) | (u64::from(result.scene_len) << 8),
            "packed ABI layout"
        );
        let command_count = {
            let runtime = resource.runtime.as_ref().expect("initialized");
            let scene = &runtime.scene[..result.scene_len as usize];
            SceneCmds::new(scene).expect("decodable scene").count()
        };
        assert!(command_count > 10);
    }

    let invalid_panel = unpack(resource.render_result(99));
    assert_eq!(invalid_panel.status, RenderStatus::InvalidPanel as u32);
    assert_eq!(invalid_panel.scene_len, 0);
    assert_eq!(invalid_panel.generation, 0);
    assert_eq!(
        resource.runtime.as_ref().expect("initialized").generation,
        [1, 1]
    );

    {
        let runtime = resource.runtime.as_mut().expect("initialized");
        runtime.state[0..4].copy_from_slice(&99u32.to_le_bytes());
    }
    let failed_after_success = unpack(resource.render_result(0));
    assert_eq!(
        failed_after_success.status,
        RenderStatus::StateBadVersion as u32
    );
    assert_eq!(failed_after_success.scene_len, 0);
    assert_eq!(failed_after_success.generation, 1);

    let runtime = resource.runtime.as_mut().expect("initialized");
    runtime.generation[0] = u32::MAX;
    write_state(runtime, &attitude_state());
    let wrapped = unpack(resource.render_result(0));
    assert_eq!(wrapped.status, RenderStatus::Ok as u32);
    assert!(wrapped.scene_len > 1);
    assert_eq!(wrapped.generation, 0, "generation wraps on success");
}

#[test]
fn resources_are_independent_and_reinitialization_resets_one() {
    let mut first = InstrumentRuntime::new();
    let mut second = InstrumentRuntime::new();
    assert_eq!(first.init(), 1);
    assert_eq!(second.init(), 1);
    write_state(
        first.runtime.as_mut().expect("initialized"),
        &attitude_state(),
    );
    assert_eq!(unpack(first.render_result(0)).generation, 1);
    let untouched = unpack(second.render_result(0));
    assert_eq!(untouched.status, RenderStatus::StateBadVersion as u32);
    assert_eq!(untouched.generation, 0);

    assert_eq!(first.init(), 1);
    let reinitialized = unpack(first.render_result(0));
    assert_eq!(reinitialized.status, RenderStatus::StateBadVersion as u32);
    assert_eq!(reinitialized.scene_len, 0);
    assert_eq!(reinitialized.generation, 0);
    assert_eq!(
        second.runtime.as_ref().expect("initialized").generation,
        [0, 0],
        "reinitializing one resource cannot mutate another"
    );
}

#[test]
fn render_into_reports_buffer_and_truncation_failures() {
    let mut tiny = Runtime {
        state: encoded_state_block(&attitude_state()),
        scene: vec![0u8; 4],
        generation: [0; 2],
        pfd_cfg: PfdConfig::default(),
    };
    assert_attempt(
        render_into(&mut tiny, 0),
        RenderStatus::SceneBufferFull,
        0,
        0,
    );
    assert_eq!(tiny.generation, [0; 2], "failure must not advance");

    let mut truncated = Runtime {
        state: encoded_state_block(&attitude_state())[..STATE_ABI_SIZE - 1].to_vec(),
        scene: vec![0u8; SCENE_CAPACITY],
        generation: [4, 8],
        pfd_cfg: PfdConfig::default(),
    };
    assert_attempt(
        render_into(&mut truncated, 0),
        RenderStatus::StateTruncated,
        0,
        4,
    );

    let mut valid = Runtime {
        state: encoded_state_block(&attitude_state()),
        scene: vec![0u8; SCENE_CAPACITY],
        generation: [0; 2],
        pfd_cfg: PfdConfig::default(),
    };
    assert_attempt(render_into(&mut valid, 2), RenderStatus::InvalidPanel, 0, 0);
    let rendered = render_into(&mut valid, 0);
    assert_eq!(rendered.status, RenderStatus::Ok);
    assert!(rendered.scene_len > 1);
    assert_eq!(rendered.generation, 1);
    assert_eq!(valid.generation, [1, 0]);
}

#[test]
fn malformed_scene_never_advances_or_commits_length() {
    let mut runtime = Runtime {
        state: encoded_state_block(&attitude_state()),
        scene: vec![1, 0],
        generation: [7, 11],
        pfd_cfg: PfdConfig::default(),
    };
    assert_attempt(
        validate_and_commit_scene(&mut runtime, 0, 2),
        RenderStatus::SceneStructure,
        0,
        7,
    );
    assert_eq!(runtime.generation, [7, 11]);
}

#[test]
fn every_encode_error_maps_to_its_own_status() {
    use pilotage_instrument_scene::SceneError;

    use crate::exports::scene_error_status;
    use crate::render_status::RenderStatus;

    // Buffer exhaustion and per-command limits are different operator
    // diagnoses (capacity budget vs panel defect) and must not collapse.
    assert_eq!(
        scene_error_status(SceneError::BufferFull),
        RenderStatus::SceneBufferFull
    );
    assert_eq!(
        scene_error_status(SceneError::TooManyPoints),
        RenderStatus::SceneCommandLimit
    );
    assert_eq!(
        scene_error_status(SceneError::TextTooLong),
        RenderStatus::SceneCommandLimit
    );
}
