#![allow(clippy::expect_used, clippy::panic)]

use indicate_instrument_scene::{
    LayerId, MAX_LAYER_COMMANDS, MAX_SCENE_BYTES, SceneCmds, SceneWriter,
};
use indicate_instrument_state::abi::v7::{CAPACITY, VERSION, encode_state};
use indicate_instrument_state::{AircraftState, Attitude, Quat, Stamped};

use crate::runtime::SCENE_CAPACITY;
use crate::{RenderOutcome, RenderStatus, Runtime, abi_version, scene_error_status};

pub(crate) fn write_state(runtime: &mut Runtime, state: &AircraftState) {
    runtime.state.fill(0);
    encode_state(state, &mut runtime.state).expect("encodes");
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
        quality: indicate_instrument_state::EstimateQuality::Good,
        valid: indicate_instrument_state::ValidFlags {
            attitude: true,
            rates: true,
            position: true,
            velocity_horizontal: true,
            velocity_vertical: true,
            ..Default::default()
        },
        ..AircraftState::default()
    }
}

pub(crate) fn encoded_state_block(state: &AircraftState) -> Vec<u8> {
    let mut block = vec![0u8; CAPACITY];
    let len = encode_state(state, &mut block).expect("encodes");
    block.truncate(len);
    block
}

fn assert_outcome(outcome: RenderOutcome, status: RenderStatus, scene_len: u32, generation: u32) {
    assert_eq!(outcome.status, status);
    assert_eq!(outcome.scene_len, scene_len);
    assert_eq!(outcome.generation, generation);
}

fn scratch_runtime(state: Vec<u8>, scene: Vec<u8>, generation: Vec<u32>) -> Runtime {
    let panels = generation.len();
    Runtime {
        state,
        scene,
        generation,
        config: vec![Vec::new(); panels],
        unknown_groups: 0,
        extended_groups: 0,
        unusual: indicate_instrument_state::UnusualAttitudeState::default(),
        profile: indicate_instrument_state::AirframeDisplayProfile::simulator(),
        alerts: indicate_alerts::AlertManager::new(),
        alert_profile: indicate_alerts::AlertProfile::simulator(),
        alert_output: None,
    }
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
    scratch_runtime(encoded_state_block(&attitude_state()), buffer, vec![7, 11])
}

fn assert_scene_rejected(panel_idx: usize, scene: &[u8], expected: RenderStatus) {
    let mut runtime = scene_runtime(scene);
    let generations = runtime.generation.clone();
    let expected_generation = runtime.generation.get(panel_idx).copied().unwrap_or(0);
    let outcome = runtime.validate_and_commit_scene(panel_idx, scene.len());
    assert_outcome(outcome, expected, 0, expected_generation);
    assert_eq!(runtime.generation, generations, "failure must not advance");
}

#[test]
fn runtime_renders_each_panel_and_advances_generation_on_success() {
    assert_eq!(abi_version(), u32::from(VERSION));
    assert_eq!(Runtime::state_capacity(), CAPACITY);
    let mut runtime = Runtime::new();
    let bad_version = runtime.render(0);
    assert_outcome(bad_version, RenderStatus::StateBadVersion, 0, 0);

    write_state(&mut runtime, &attitude_state());
    for panel in [0u32, 1] {
        let outcome = runtime.render(panel);
        assert_eq!(outcome.status, RenderStatus::Ok);
        assert!(outcome.scene_len > 1, "panel {panel} rendered no scene");
        assert_eq!(outcome.generation, 1);
        let scene = &runtime.scene[..outcome.scene_len as usize];
        let command_count = SceneCmds::new(scene).expect("decodable scene").count();
        assert!(command_count > 10);
    }

    let invalid_panel = runtime.render(99);
    assert_outcome(invalid_panel, RenderStatus::InvalidPanel, 0, 0);
    assert_eq!(runtime.generation, [1, 1, 0]);

    runtime.state[0..4].copy_from_slice(&99u32.to_le_bytes());
    let failed_after_success = runtime.render(0);
    assert_outcome(failed_after_success, RenderStatus::StateBadVersion, 0, 1);

    runtime.generation[0] = u32::MAX;
    write_state(&mut runtime, &attitude_state());
    let wrapped = runtime.render(0);
    assert_eq!(wrapped.status, RenderStatus::Ok);
    assert!(wrapped.scene_len > 1);
    assert_eq!(wrapped.generation, 0, "generation wraps on success");
}

#[test]
fn runtimes_are_independent() {
    let mut first = Runtime::new();
    let mut second = Runtime::new();
    write_state(&mut first, &attitude_state());
    assert_eq!(first.render(0).generation, 1);
    let untouched = second.render(0);
    assert_eq!(untouched.status, RenderStatus::StateBadVersion);
    assert_eq!(untouched.generation, 0);
    assert_eq!(
        second.generation,
        [0, 0, 0],
        "one runtime cannot mutate another"
    );
}

#[test]
fn configuration_endpoints_validate_and_apply() {
    let mut runtime = Runtime::new();
    assert_eq!(
        runtime.set_v_speeds(40.0, 48.0, 85.0, 129.0, 163.0),
        RenderStatus::Ok
    );
    assert_eq!(
        runtime.set_panel_config(99, &[]),
        RenderStatus::InvalidPanel
    );
    assert_eq!(
        runtime.set_panel_config(0, &[1, 0, 9]),
        RenderStatus::ConfigInvalid,
        "a malformed blob is refused"
    );
}

#[test]
fn a_non_canonical_state_frame_fails_malformed() {
    // Tags out of ascending order must fail with the malformed-state
    // status, not truncation and never a render.
    let mut frame = vec![7u8, 2];
    frame.extend_from_slice(&[0x05, 12, 0]);
    frame.extend_from_slice(&[0u8; 12]);
    frame.extend_from_slice(&[0x03, 12, 0]);
    frame.extend_from_slice(&[0u8; 12]);
    let mut malformed = scratch_runtime(frame, vec![0u8; SCENE_CAPACITY], vec![2, 3]);
    assert_outcome(malformed.render(0), RenderStatus::StateMalformed, 0, 2);
}

#[test]
fn render_reports_buffer_and_truncation_failures() {
    let mut tiny = scratch_runtime(
        encoded_state_block(&attitude_state()),
        vec![0u8; 4],
        vec![0; 2],
    );
    assert_outcome(tiny.render(0), RenderStatus::SceneBufferFull, 0, 0);
    assert_eq!(tiny.generation, [0; 2], "failure must not advance");

    let frame = encoded_state_block(&attitude_state());
    let mut truncated = scratch_runtime(
        frame[..frame.len() - 1].to_vec(),
        vec![0u8; SCENE_CAPACITY],
        vec![4, 8],
    );
    assert_outcome(truncated.render(0), RenderStatus::StateTruncated, 0, 4);

    let mut valid = scratch_runtime(
        encoded_state_block(&attitude_state()),
        vec![0u8; SCENE_CAPACITY],
        vec![0; 2],
    );
    assert_outcome(valid.render(2), RenderStatus::InvalidPanel, 0, 0);
    let rendered = valid.render(0);
    assert_eq!(rendered.status, RenderStatus::Ok);
    assert!(rendered.scene_len > 1);
    assert_eq!(rendered.generation, 1);
    assert_eq!(valid.generation, [1, 0]);
}

#[test]
fn malformed_scene_never_advances_or_commits_length() {
    let mut runtime = scratch_runtime(
        encoded_state_block(&attitude_state()),
        vec![1, 0],
        vec![7, 11],
    );
    let outcome = runtime.validate_and_commit_scene(0, 2);
    assert_outcome(outcome, RenderStatus::SceneStructure, 0, 7);
    assert_eq!(runtime.generation, [7, 11]);
}

#[test]
fn every_encode_error_maps_to_its_own_status() {
    use indicate_instrument_scene::SceneError;

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
    let pfd = panel_scene(&[
        LayerId::Attitude,
        LayerId::Tapes,
        LayerId::Guidance,
        LayerId::Annunciation,
    ]);
    let hsi = panel_scene(&[
        LayerId::Attitude,
        LayerId::Tapes,
        LayerId::Guidance,
        LayerId::Annunciation,
    ]);
    for (panel_idx, scene, expected_generation) in [(0, pfd, [8, 11]), (1, hsi, [7, 12])] {
        let mut runtime = scene_runtime(&scene);
        let outcome = runtime.validate_and_commit_scene(panel_idx, scene.len());
        assert_outcome(
            outcome,
            RenderStatus::Ok,
            scene.len() as u32,
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

    let pfd_missing_annunciation =
        panel_scene(&[LayerId::Attitude, LayerId::Tapes, LayerId::Guidance]);
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
fn glyph_accessors_surface_the_verified_pack() {
    use indicate_instrument_glyphs::PANEL_GLYPHS;

    use crate::{glyph_manifest, glyph_recorded_hash};

    assert!(PANEL_GLYPHS.verify().is_ok(), "shipped pack verifies");
    let canonical = glyph_manifest();
    assert_eq!(canonical.len(), PANEL_GLYPHS.canonical_len());
    let mut expected = vec![0u8; PANEL_GLYPHS.canonical_len()];
    let len = PANEL_GLYPHS
        .write_canonical(&mut expected)
        .expect("canonical fits");
    assert_eq!(canonical, expected[..len]);
    assert_eq!(glyph_recorded_hash(), PANEL_GLYPHS.recorded_hash().to_vec());
}

#[test]
fn compatibility_pin_matches_linked_runtime() {
    let pin: serde_json::Value = serde_json::from_str(include_str!(
        "../../../clients/instrument-compatibility.json"
    ))
    .expect("compatibility pin is valid JSON");
    assert_eq!(pin["stateAbiVersion"], crate::abi_version());
    assert_eq!(pin["sceneFormatVersion"], crate::scene_format_version());
    assert_eq!(pin["corpusVersion"], crate::corpus_version());
    assert_eq!(pin["corpusDigest"], crate::corpus_digest_hex());
    assert_eq!(pin["registrySceneDigest"], crate::scene_digest_hex());
    assert_eq!(
        pin["screenCompositionDigest"],
        crate::composition_digest_hex()
    );
}
