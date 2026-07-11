#![allow(clippy::expect_used, clippy::panic)]

use pilotage_instrument_panels::PfdConfig;
use pilotage_instrument_scene::SceneCmds;
use pilotage_instrument_state::abi::{STATE_ABI_SIZE, STATE_ABI_VERSION, encode_state};
use pilotage_instrument_state::{AircraftState, Attitude, Quat, Stamped};

use super::{
    RenderStatus, abi_version, init, render_generation, render_status, scene_len, scene_ptr,
    state_len, state_ptr,
};
use crate::exports::{Ctx, render_into};

fn write_state(state: &AircraftState) {
    let mut block = vec![0u8; STATE_ABI_SIZE];
    encode_state(state, &mut block).expect("encodes");
    // Outside WASM there is no shared linear memory; poke the context
    // buffer through the same path render reads.
    let Ok(mut guard) = crate::exports::CTX.lock() else {
        panic!("ctx lock");
    };
    let ctx = guard.as_mut().expect("init called");
    ctx.state.copy_from_slice(&block);
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

// One test function: the context is a process-global, so separate tests
// would race on it.
#[test]
fn exported_surface_reports_status_and_generation() {
    assert_eq!(abi_version(), STATE_ABI_VERSION);

    // Before init there is nothing to render into.
    assert_eq!(render_status(0), RenderStatus::NotInitialized as u32);

    assert_eq!(init(), 1);
    assert_ne!(state_ptr(), 0);
    assert_ne!(scene_ptr(), 0);
    assert_eq!(state_len() as usize, STATE_ABI_SIZE);
    assert_eq!(scene_len(), 0, "no scene before the first success");

    // The zeroed state block carries version 0: a decode failure, and a
    // distinct code from a truncated block.
    assert_eq!(render_status(0), RenderStatus::StateBadVersion as u32);
    assert_eq!(render_generation(0), 0, "failure must not advance");

    write_state(&attitude_state());
    for (panel, expected_generation) in [(0u32, 1u32), (1, 1)] {
        assert_eq!(render_status(panel), RenderStatus::Ok as u32);
        assert_eq!(render_generation(panel), expected_generation);
        let len = scene_len();
        assert!(len > 1, "panel {panel} rendered {len} bytes");
        let Ok(guard) = crate::exports::CTX.lock() else {
            panic!("ctx lock");
        };
        let ctx = guard.as_ref().expect("ctx");
        let scene = &ctx.scene[..len as usize];
        let cmds = SceneCmds::new(scene).expect("decodable scene");
        assert!(cmds.count() > 10);
    }

    // Unknown panel ids are a typed failure, not a quiet zero, and leave
    // the last good scene length and generations untouched.
    let good_len = scene_len();
    assert_eq!(render_status(99), RenderStatus::InvalidPanel as u32);
    assert_eq!(scene_len(), good_len);
    assert_eq!(render_generation(0), 1);
    assert_eq!(render_generation(1), 1);
    assert_eq!(render_generation(99), 0, "unknown panel has no generation");

    // A failed decode after a success keeps the failure visible in the
    // status while preserving the prior generation (no silent advance).
    {
        let Ok(mut guard) = crate::exports::CTX.lock() else {
            panic!("ctx lock");
        };
        let ctx = guard.as_mut().expect("ctx");
        ctx.state[0..4].copy_from_slice(&99u32.to_le_bytes());
    }
    assert_eq!(render_status(0), RenderStatus::StateBadVersion as u32);
    assert_eq!(render_generation(0), 1);

    // Generation wraps rather than panicking or saturating.
    {
        let Ok(mut guard) = crate::exports::CTX.lock() else {
            panic!("ctx lock");
        };
        let ctx = guard.as_mut().expect("ctx");
        ctx.generation[0] = u32::MAX;
    }
    write_state(&attitude_state());
    assert_eq!(render_status(0), RenderStatus::Ok as u32);
    assert_eq!(render_generation(0), 0, "wrapped, still an advance");
}

#[test]
fn render_into_reports_buffer_and_truncation_failures() {
    // A scene buffer too small for even the version byte + first command.
    let mut tiny = Ctx {
        state: encoded_state_block(&attitude_state()),
        scene: vec![0u8; 4],
        scene_len: 0,
        generation: [0; 2],
        pfd_cfg: PfdConfig::default(),
    };
    assert_eq!(render_into(&mut tiny, 0), RenderStatus::SceneBufferFull);
    assert_eq!(tiny.generation, [0; 2], "failure must not advance");
    assert_eq!(tiny.scene_len, 0);

    // A state block shorter than the ABI decodes as truncated.
    let mut truncated = Ctx {
        state: encoded_state_block(&attitude_state())[..STATE_ABI_SIZE - 1].to_vec(),
        scene: vec![0u8; 64 * 1024],
        scene_len: 0,
        generation: [0; 2],
        pfd_cfg: PfdConfig::default(),
    };
    assert_eq!(render_into(&mut truncated, 0), RenderStatus::StateTruncated);

    // Invalid panel is rejected before any state or scene work.
    let mut ok_ctx = Ctx {
        state: encoded_state_block(&attitude_state()),
        scene: vec![0u8; 64 * 1024],
        scene_len: 0,
        generation: [0; 2],
        pfd_cfg: PfdConfig::default(),
    };
    assert_eq!(render_into(&mut ok_ctx, 2), RenderStatus::InvalidPanel);
    assert_eq!(render_into(&mut ok_ctx, 0), RenderStatus::Ok);
    assert!(ok_ctx.scene_len > 1);
    assert_eq!(ok_ctx.generation, [1, 0]);
}
