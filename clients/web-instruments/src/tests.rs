#![allow(clippy::expect_used, clippy::panic)]

use pilotage_instrument_panels::PfdConfig;
use pilotage_instrument_scene::{
    LayerId, MAX_LAYER_COMMANDS, MAX_SCENE_BYTES, SceneCmds, SceneWriter,
};
use pilotage_instrument_state::abi::{STATE_ABI_SIZE, STATE_ABI_VERSION, encode_state};
use pilotage_instrument_state::{AircraftState, Attitude, Quat, Stamped};

use super::{InstrumentRuntime, RenderStatus, abi_version};
use crate::exports::{
    RenderAttempt, Runtime, SCENE_CAPACITY, render_into, validate_and_commit_scene,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PackedResult {
    pub(crate) status: u32,
    pub(crate) scene_len: u32,
    pub(crate) generation: u32,
}

pub(crate) fn unpack(result: u64) -> PackedResult {
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

pub(crate) fn attitude_state() -> AircraftState {
    AircraftState {
        attitude: Stamped {
            data: Some(Attitude {
                quat: Quat::IDENTITY,
                rates_rps: [0.0; 3],
            }),
            age_ms: Some(10.0),
        },
        quality: pilotage_instrument_state::EstimateQuality::Good,
        valid: pilotage_instrument_state::ValidFlags {
            attitude: true,
            rates: true,
            position: true,
            velocity: true,
        },
        ..AircraftState::default()
    }
}

pub(crate) fn encoded_state_block(state: &AircraftState) -> Vec<u8> {
    let mut block = vec![0u8; STATE_ABI_SIZE];
    encode_state(state, &mut block).expect("encodes");
    block
}

fn assert_attempt(attempt: RenderAttempt, status: RenderStatus, scene_len: usize, generation: u32) {
    assert_eq!(attempt.status, status);
    assert_eq!(attempt.scene_len, scene_len);
    assert_eq!(attempt.generation, generation);
}

fn encoded_scene(build: impl FnOnce(&mut SceneWriter<'_>)) -> Vec<u8> {
    let mut scene = vec![0u8; MAX_SCENE_BYTES];
    let mut writer = SceneWriter::new(&mut scene).expect("writer");
    build(&mut writer);
    let len = writer.finish();
    scene.truncate(len);
    scene
}

fn simple_layer(writer: &mut SceneWriter<'_>, layer: LayerId) {
    writer.begin_layer(layer).expect("begin layer");
    writer.line(0.0, 0.0, 1.0, 1.0).expect("line");
    writer.end_layer(layer).expect("end layer");
}

fn panel_scene(layers: &[LayerId]) -> Vec<u8> {
    encoded_scene(|writer| {
        for layer in layers {
            simple_layer(writer, *layer);
        }
    })
}

fn scene_runtime(scene: &[u8]) -> Runtime {
    let mut buffer = vec![0u8; scene.len().max(MAX_SCENE_BYTES)];
    buffer[..scene.len()].copy_from_slice(scene);
    Runtime {
        state: encoded_state_block(&attitude_state()),
        scene: buffer,
        generation: [7, 11],
        pfd_cfg: PfdConfig::default(),
        unusual: pilotage_instrument_state::UnusualAttitudeState::default(),
        profile: pilotage_instrument_state::AirframeDisplayProfile::simulator(),
        alerts: pilotage_alerts::AlertManager::new(),
        alert_profile: pilotage_alerts::AlertProfile::simulator(),
        alert_output: None,
    }
}

fn assert_scene_rejected(panel_idx: usize, scene: &[u8], expected: RenderStatus) {
    let mut runtime = scene_runtime(scene);
    let generations = runtime.generation;
    let expected_generation = runtime.generation.get(panel_idx).copied().unwrap_or(0);
    let attempt = validate_and_commit_scene(&mut runtime, panel_idx, scene.len());
    assert_attempt(attempt, expected, 0, expected_generation);
    assert_eq!(runtime.generation, generations, "failure must not advance");
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
        unusual: pilotage_instrument_state::UnusualAttitudeState::default(),
        profile: pilotage_instrument_state::AirframeDisplayProfile::simulator(),
        alerts: pilotage_alerts::AlertManager::new(),
        alert_profile: pilotage_alerts::AlertProfile::simulator(),
        alert_output: None,
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
        unusual: pilotage_instrument_state::UnusualAttitudeState::default(),
        profile: pilotage_instrument_state::AirframeDisplayProfile::simulator(),
        alerts: pilotage_alerts::AlertManager::new(),
        alert_profile: pilotage_alerts::AlertProfile::simulator(),
        alert_output: None,
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
        unusual: pilotage_instrument_state::UnusualAttitudeState::default(),
        profile: pilotage_instrument_state::AirframeDisplayProfile::simulator(),
        alerts: pilotage_alerts::AlertManager::new(),
        alert_profile: pilotage_alerts::AlertProfile::simulator(),
        alert_output: None,
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
        unusual: pilotage_instrument_state::UnusualAttitudeState::default(),
        profile: pilotage_instrument_state::AirframeDisplayProfile::simulator(),
        alerts: pilotage_alerts::AlertManager::new(),
        alert_profile: pilotage_alerts::AlertProfile::simulator(),
        alert_output: None,
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

#[test]
fn critical_layer_masks_gate_visible_commit() {
    let pfd = panel_scene(&[LayerId::Attitude, LayerId::Tapes, LayerId::Annunciation]);
    let hsi = panel_scene(&[
        LayerId::Attitude,
        LayerId::Tapes,
        LayerId::Guidance,
        LayerId::Annunciation,
    ]);
    for (panel_idx, scene, expected_generation) in [(0, pfd, [8, 11]), (1, hsi, [7, 12])] {
        let mut runtime = scene_runtime(&scene);
        let attempt = validate_and_commit_scene(&mut runtime, panel_idx, scene.len());
        assert_attempt(
            attempt,
            RenderStatus::Ok,
            scene.len(),
            expected_generation[panel_idx],
        );
        assert_eq!(runtime.generation, expected_generation);
    }

    let background_only = panel_scene(&[LayerId::Background]);
    let failure_only = panel_scene(&[LayerId::Failure]);
    assert_scene_rejected(
        0,
        &background_only,
        RenderStatus::SceneCriticalLayersMissing,
    );
    assert_scene_rejected(1, &failure_only, RenderStatus::SceneCriticalLayersMissing);

    let pfd_missing_annunciation = panel_scene(&[LayerId::Attitude, LayerId::Tapes]);
    let hsi_missing_guidance =
        panel_scene(&[LayerId::Attitude, LayerId::Tapes, LayerId::Annunciation]);
    assert_scene_rejected(
        0,
        &pfd_missing_annunciation,
        RenderStatus::SceneCriticalLayersMissing,
    );
    assert_scene_rejected(
        1,
        &hsi_missing_guidance,
        RenderStatus::SceneCriticalLayersMissing,
    );
}

#[test]
fn layer_order_ownership_and_nesting_gate_visible_commit() {
    let duplicate = panel_scene(&[LayerId::Attitude, LayerId::Attitude]);
    let out_of_order = panel_scene(&[LayerId::Tapes, LayerId::Attitude]);
    let nested = encoded_scene(|writer| {
        writer.begin_layer(LayerId::Attitude).expect("outer layer");
        writer.begin_layer(LayerId::Tapes).expect("nested layer");
    });
    for scene in [duplicate, out_of_order, nested] {
        assert_scene_rejected(0, &scene, RenderStatus::SceneLayerContract);
    }
}

#[test]
fn layer_state_and_budgets_gate_visible_commit() {
    let unbalanced = encoded_scene(|writer| {
        writer.begin_layer(LayerId::Attitude).expect("begin layer");
        writer.save().expect("nested state");
        writer.end_layer(LayerId::Attitude).expect("end layer");
    });
    assert_scene_rejected(0, &unbalanced, RenderStatus::SceneLayerContract);

    let over_budget = encoded_scene(|writer| {
        writer.begin_layer(LayerId::Attitude).expect("begin layer");
        for _ in 0..(MAX_LAYER_COMMANDS - 2) / 2 {
            writer.save().expect("save");
            writer.restore().expect("restore");
        }
        writer.rotate(0.1).expect("over-budget command");
        writer.end_layer(LayerId::Attitude).expect("end layer");
    });
    assert_scene_rejected(0, &over_budget, RenderStatus::SceneLayerContract);

    let oversized = vec![0u8; MAX_SCENE_BYTES + 1];
    assert_scene_rejected(0, &oversized, RenderStatus::SceneLayerContract);
}

#[test]
fn malformed_scene_framing_gates_visible_commit() {
    let mut truncated = panel_scene(&[LayerId::Attitude, LayerId::Tapes, LayerId::Annunciation]);
    truncated.pop();
    assert_scene_rejected(0, &truncated, RenderStatus::SceneStructure);

    let outside_layer = encoded_scene(|writer| {
        writer.line(0.0, 0.0, 1.0, 1.0).expect("line");
    });
    assert_scene_rejected(0, &outside_layer, RenderStatus::SceneLayerContract);
}

#[test]
fn glyph_exports_surface_the_verified_pack() {
    use pilotage_instrument_glyphs::PANEL_GLYPHS;

    use crate::exports::InstrumentRuntime;

    assert!(PANEL_GLYPHS.verify().is_ok(), "shipped pack verifies");
    let runtime = InstrumentRuntime::new();
    let canonical = runtime.glyph_manifest();
    assert_eq!(canonical.len(), PANEL_GLYPHS.canonical_len());
    let mut expected = vec![0u8; PANEL_GLYPHS.canonical_len()];
    let len = PANEL_GLYPHS
        .write_canonical(&mut expected)
        .expect("canonical fits");
    assert_eq!(canonical, expected[..len]);
    assert_eq!(
        runtime.glyph_recorded_hash(),
        PANEL_GLYPHS.recorded_hash().to_vec()
    );
}
