#![allow(clippy::expect_used, clippy::panic)]

use crate::SCENE_FORMAT_VERSION;
use crate::cmd::{Anchor, Cmd, PaintMode};
use crate::color::Rgba8;
use crate::decode::SceneCmds;
use crate::encode::{SceneError, SceneWriter};

fn decode_all(scene: &[u8]) -> alloc_free_vec::CmdVec<'_> {
    let cmds = SceneCmds::new(scene).expect("valid scene header");
    let mut out = alloc_free_vec::CmdVec::new();
    for c in cmds {
        out.push(c.expect("valid command"));
    }
    out
}

/// A tiny fixed-capacity Vec so tests stay alloc-free like the crate.
mod alloc_free_vec {
    use crate::cmd::Cmd;

    pub struct CmdVec<'a> {
        items: [Option<Cmd<'a>>; 64],
        len: usize,
    }

    impl<'a> CmdVec<'a> {
        pub fn new() -> Self {
            Self {
                items: [const { None }; 64],
                len: 0,
            }
        }

        pub fn push(&mut self, c: Cmd<'a>) {
            assert!(self.len < 64, "test scene too large");
            self.items[self.len] = Some(c);
            self.len += 1;
        }

        pub fn len(&self) -> usize {
            self.len
        }

        pub fn get(&self, i: usize) -> &Cmd<'a> {
            self.items[i].as_ref().expect("index within len")
        }
    }
}

#[test]
fn header_is_version_byte() {
    let mut buf = [0u8; 16];
    let w = SceneWriter::new(&mut buf).expect("fits");
    assert_eq!(w.finish(), 1);
    assert_eq!(buf[0], SCENE_FORMAT_VERSION);
}

#[test]
fn every_command_round_trips() {
    let mut buf = [0u8; 512];
    let mut w = SceneWriter::new(&mut buf).expect("fits");
    w.save().expect("fits");
    w.translate(240.0, 180.0).expect("fits");
    w.rotate(0.5).expect("fits");
    w.fill_color(Rgba8::rgb(1, 2, 3)).expect("fits");
    w.stroke(Rgba8::rgba(4, 5, 6, 7), 2.5).expect("fits");
    w.line(0.0, 1.0, 2.0, 3.0).expect("fits");
    w.polyline(&[[0.0, 0.0], [1.0, 1.0]]).expect("fits");
    w.polygon(PaintMode::FillStroke, &[[0.0, 0.0], [1.0, 0.0], [0.5, 1.0]])
        .expect("fits");
    w.rect(PaintMode::Fill, 1.0, 2.0, 3.0, 4.0).expect("fits");
    w.circle(PaintMode::Stroke, 5.0, 6.0, 7.0).expect("fits");
    w.arc(0.0, 0.0, 144.0, 0.5, 2.0).expect("fits");
    w.text(10.0, 20.0, 18.0, Anchor::CENTER, "074")
        .expect("fits");
    w.clip_rect(0.0, 0.0, 480.0, 360.0).expect("fits");
    w.restore().expect("fits");
    let len = w.finish();

    let cmds = decode_all(&buf[..len]);
    assert_eq!(cmds.len(), 14);
    assert_eq!(*cmds.get(0), Cmd::Save);
    assert_eq!(*cmds.get(1), Cmd::Translate { x: 240.0, y: 180.0 });
    assert_eq!(*cmds.get(2), Cmd::Rotate { radians: 0.5 });
    assert_eq!(
        *cmds.get(3),
        Cmd::FillColor {
            color: Rgba8::rgb(1, 2, 3)
        }
    );
    match cmds.get(7) {
        Cmd::Polygon { mode, points } => {
            assert_eq!(*mode, PaintMode::FillStroke);
            assert_eq!(points.len(), 3);
            assert_eq!(points.get(2), Some([0.5, 1.0]));
        }
        other => panic!("expected polygon, got {other:?}"),
    }
    match cmds.get(11) {
        Cmd::Text {
            x,
            y,
            size,
            anchor,
            text,
        } => {
            assert_eq!((*x, *y, *size), (10.0, 20.0, 18.0));
            assert_eq!(*anchor, Anchor::CENTER);
            assert_eq!(*text, "074");
        }
        other => panic!("expected text, got {other:?}"),
    }
    assert_eq!(*cmds.get(13), Cmd::Restore);
}

#[test]
fn overflowing_command_rolls_back_whole() {
    let mut buf = [0u8; 12];
    let mut w = SceneWriter::new(&mut buf).expect("fits");
    w.line(0.0, 1.0, 2.0, 3.0)
        .expect_err("line needs 19 bytes, only 11 remain");
    // The failed command must leave no partial bytes behind.
    assert_eq!(w.len(), 1);
    w.save().expect("small command still fits after rollback");
}

#[test]
fn text_over_limit_is_rejected() {
    let mut buf = [0u8; 1024];
    let mut w = SceneWriter::new(&mut buf).expect("fits");
    let long = core::str::from_utf8(&[b'x'; 251]).expect("ascii");
    assert_eq!(
        w.text(0.0, 0.0, 12.0, Anchor::BASELINE_LEFT, long),
        Err(SceneError::TextTooLong)
    );
}

#[test]
fn unknown_opcode_is_skipped_not_fatal() {
    let mut buf = [0u8; 64];
    let mut w = SceneWriter::new(&mut buf).expect("fits");
    w.save().expect("fits");
    let mut len = w.finish();
    // Splice in a future opcode (0x7F) with a 4-byte payload, then a
    // trailing Restore, as a newer encoder would produce.
    buf[len..len + 7].copy_from_slice(&[0x7f, 4, 0, 0xde, 0xad, 0xbe, 0xef]);
    len += 7;
    buf[len..len + 3].copy_from_slice(&[0x02, 0, 0]);
    len += 3;

    let cmds = decode_all(&buf[..len]);
    assert_eq!(cmds.len(), 3);
    assert_eq!(*cmds.get(1), Cmd::Unknown { opcode: 0x7f });
    assert_eq!(*cmds.get(2), Cmd::Restore);
}
