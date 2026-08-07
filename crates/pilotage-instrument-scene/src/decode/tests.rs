#![allow(clippy::expect_used, clippy::panic)]

use crate::Cmd;
use crate::SCENE_FORMAT_VERSION;
use crate::decode::{DecodeError, SceneCmds};

#[test]
fn empty_input_is_truncated() {
    assert_eq!(SceneCmds::new(&[]).err(), Some(DecodeError::Truncated));
}

#[test]
fn wrong_version_is_rejected() {
    assert_eq!(
        SceneCmds::new(&[99]).err(),
        Some(DecodeError::BadVersion { found: 99 })
    );
}

#[test]
fn truncated_command_is_an_error_then_stops() {
    // Version byte, then a LINE opcode claiming 16 payload bytes with
    // only 2 present.
    let bytes = [SCENE_FORMAT_VERSION, 0x20, 16, 0, 1, 2];
    let mut cmds = SceneCmds::new(&bytes).expect("header ok");
    assert_eq!(cmds.next(), Some(Err(DecodeError::Truncated)));
    assert_eq!(cmds.next(), None);
}

#[test]
fn bad_payload_names_the_opcode() {
    // RECT with an empty payload: known opcode, malformed body.
    let bytes = [SCENE_FORMAT_VERSION, 0x23, 0, 0];
    let mut cmds = SceneCmds::new(&bytes).expect("header ok");
    assert_eq!(
        cmds.next(),
        Some(Err(DecodeError::BadPayload { opcode: 0x23 }))
    );
}

#[test]
fn empty_scene_yields_no_commands() {
    let bytes = [SCENE_FORMAT_VERSION];
    let mut cmds = SceneCmds::new(&bytes).expect("header ok");
    assert_eq!(cmds.next(), None);
}

#[test]
fn an_attributed_text_run_round_trips_as_the_claim_then_the_run() {
    use crate::{Anchor, SceneWriter};
    let mut buf = [0u8; 256];
    let mut w = SceneWriter::new(&mut buf).expect("fits");
    w.text_attributed(0x03, 120.0, 40.0, 16.0, Anchor::CENTER, "128")
        .expect("encodes");
    let used = w.finish();
    let mut cmds = SceneCmds::new(&buf[..used]).expect("header ok");
    assert_eq!(cmds.next(), Some(Ok(Cmd::Attribute { group: 0x03 })));
    match cmds.next() {
        Some(Ok(Cmd::Text { text: "128", .. })) => {}
        other => panic!("expected the attributed run, got {other:?}"),
    }
    assert_eq!(cmds.next(), None);
}

#[test]
fn a_malformed_attribute_payload_names_the_opcode() {
    // ATTRIBUTE with a two-byte payload: known opcode, malformed body.
    let bytes = [SCENE_FORMAT_VERSION, 0x31, 2, 0, 1, 2];
    let mut cmds = SceneCmds::new(&bytes).expect("header ok");
    assert_eq!(
        cmds.next(),
        Some(Err(DecodeError::BadPayload { opcode: 0x31 }))
    );
}
