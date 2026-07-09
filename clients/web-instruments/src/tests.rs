#![allow(clippy::expect_used, clippy::panic)]

use pilotage_instrument_scene::SceneCmds;
use pilotage_instrument_state::abi::{STATE_ABI_SIZE, encode_state};
use pilotage_instrument_state::{AircraftState, Attitude, Quat, Stamped};

use super::{abi_version, init, render, scene_ptr, state_len, state_ptr};

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

#[test]
fn exported_surface_renders_both_panels() {
    assert_eq!(abi_version(), 1);
    assert_eq!(init(), 1);
    assert_ne!(state_ptr(), 0);
    assert_ne!(scene_ptr(), 0);
    assert_eq!(state_len() as usize, STATE_ABI_SIZE);

    write_state(&AircraftState {
        attitude: Stamped {
            data: Some(Attitude {
                quat: Quat::IDENTITY,
                rates_rps: [0.0; 3],
            }),
            age_ms: Some(10.0),
        },
        ..AircraftState::default()
    });

    for panel in [0u32, 1] {
        let len = render(panel);
        assert!(len > 1, "panel {panel} rendered {len} bytes");
        let Ok(guard) = crate::exports::CTX.lock() else {
            panic!("ctx lock");
        };
        let ctx = guard.as_ref().expect("ctx");
        let scene = &ctx.scene[..len as usize];
        let cmds = SceneCmds::new(scene).expect("decodable scene");
        assert!(cmds.count() > 10);
    }

    // Unknown panel ids render nothing. Kept in this test because the
    // context is a process-global; separate tests would race on it.
    assert_eq!(render(99), 0);
}
