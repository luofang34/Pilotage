#![allow(clippy::expect_used, clippy::panic)]

use proptest::prelude::*;
use std::vec::Vec;

use crate::cmd::PaintMode;
use crate::decode::DecodeError;
use crate::encode::SceneWriter;
use crate::layer::{
    LAYER_COUNT, LayerError, LayerId, MAX_LAYER_COMMANDS, MAX_SCENE_BYTES, MAX_STACK_DEPTH,
    validate_layers,
};

const ALL: [LayerId; LAYER_COUNT] = [
    LayerId::Background,
    LayerId::Attitude,
    LayerId::Tapes,
    LayerId::Guidance,
    LayerId::Annunciation,
    LayerId::Failure,
];

fn scene(build: impl FnOnce(&mut SceneWriter<'_>)) -> Vec<u8> {
    let mut buf = std::vec![0u8; MAX_SCENE_BYTES];
    let mut writer = SceneWriter::new(&mut buf).expect("writer");
    build(&mut writer);
    let len = writer.finish();
    buf.truncate(len);
    buf
}

fn simple_layer(w: &mut SceneWriter<'_>, layer: LayerId) {
    w.begin_layer(layer).expect("begin");
    w.line(0.0, 0.0, 1.0, 1.0).expect("line");
    w.end_layer(layer).expect("end");
}

fn layer_without_entry_save(build: impl FnOnce(&mut SceneWriter<'_>)) -> Vec<u8> {
    let mut bytes = scene(|writer| {
        writer.begin_layer(LayerId::Background).expect("begin");
        build(writer);
        writer.end_layer(LayerId::Background).expect("end");
    });
    assert_eq!(&bytes[5..8], &[0x01, 0, 0], "mandatory entry save");
    bytes.drain(5..8);
    bytes
}

fn single_raw_command(opcode: u8, payload: &[u8]) -> Vec<u8> {
    let payload_len = u16::try_from(payload.len()).expect("test payload fits");
    let mut bytes = std::vec![crate::SCENE_FORMAT_VERSION, opcode];
    bytes.extend_from_slice(&payload_len.to_le_bytes());
    bytes.extend_from_slice(payload);
    bytes
}

#[test]
fn every_ascending_subset_is_legal() {
    for mask in 0u8..(1 << LAYER_COUNT) {
        let bytes = scene(|w| {
            for layer in ALL {
                if mask & (1 << layer.to_u8()) != 0 {
                    simple_layer(w, layer);
                }
            }
        });
        let report = validate_layers(&bytes).expect("legal subset validates");
        assert_eq!(report.present, mask, "mask {mask:#08b}");
        for layer in ALL {
            assert_eq!(report.contains(layer), mask & (1 << layer.to_u8()) != 0);
            if report.contains(layer) {
                assert_eq!(report.commands[layer.to_u8() as usize], 3);
            }
        }
    }
}

#[test]
fn every_non_ascending_pair_is_illegal() {
    for first in ALL {
        for second in ALL {
            if second > first {
                continue;
            }
            let bytes = scene(|w| {
                simple_layer(w, first);
                simple_layer(w, second);
            });
            let expected = if second == first {
                LayerError::DuplicateLayer { layer: second }
            } else {
                LayerError::OutOfOrder { layer: second }
            };
            assert_eq!(
                validate_layers(&bytes),
                Err(expected),
                "{first:?} then {second:?}"
            );
        }
    }
}

#[test]
fn structural_violations_fail_the_frame() {
    let nested = scene(|w| {
        w.begin_layer(LayerId::Background).expect("begin");
        w.begin_layer(LayerId::Attitude).expect("begin");
    });
    assert_eq!(
        validate_layers(&nested),
        Err(LayerError::NestedLayer {
            layer: LayerId::Attitude
        })
    );

    let stray_end = single_raw_command(0x51, &[LayerId::Tapes.to_u8()]);
    assert_eq!(
        validate_layers(&stray_end),
        Err(LayerError::EndWithoutBegin {
            layer: LayerId::Tapes
        })
    );

    let mismatch = scene(|w| {
        w.begin_layer(LayerId::Background).expect("begin");
        w.end_layer(LayerId::Attitude).expect("end");
    });
    assert_eq!(
        validate_layers(&mismatch),
        Err(LayerError::EndMismatch {
            open: LayerId::Background,
            end: LayerId::Attitude,
        })
    );

    let unclosed = scene(|w| {
        w.begin_layer(LayerId::Background).expect("begin");
        w.line(0.0, 0.0, 1.0, 1.0).expect("line");
    });
    assert_eq!(
        validate_layers(&unclosed),
        Err(LayerError::UnclosedLayer {
            layer: LayerId::Background
        })
    );

    let outside = scene(|w| {
        w.line(0.0, 0.0, 1.0, 1.0).expect("line");
    });
    assert_eq!(
        validate_layers(&outside),
        Err(LayerError::CommandOutsideLayer)
    );
}

#[test]
fn state_leaks_across_layers_fail_the_frame() {
    // A save left open at the layer's end would carry its transform and
    // clip into every band above it.
    let open_save = scene(|w| {
        w.begin_layer(LayerId::Background).expect("begin");
        w.save().expect("save");
        w.clip_rect(0.0, 0.0, 1.0, 1.0).expect("clip");
        w.end_layer(LayerId::Background).expect("end");
    });
    assert_eq!(
        validate_layers(&open_save),
        Err(LayerError::UnbalancedState {
            layer: LayerId::Background
        })
    );

    // A restore below the entry depth would pop state a *lower* band
    // established for itself.
    let deep_restore = scene(|w| {
        w.begin_layer(LayerId::Background).expect("begin");
        w.restore().expect("restore");
        w.end_layer(LayerId::Background).expect("end");
    });
    assert_eq!(
        validate_layers(&deep_restore),
        Err(LayerError::UnbalancedState {
            layer: LayerId::Background
        })
    );

    let balanced = scene(|w| {
        w.begin_layer(LayerId::Background).expect("begin");
        w.save().expect("save");
        w.rotate(1.0).expect("rotate");
        w.restore().expect("restore");
        w.end_layer(LayerId::Background).expect("end");
    });
    assert!(validate_layers(&balanced).is_ok());

    let base_state_leaks = [
        layer_without_entry_save(|w| w.translate(10.0, 20.0).expect("translate")),
        layer_without_entry_save(|w| w.clip_rect(0.0, 0.0, 1.0, 1.0).expect("clip")),
        layer_without_entry_save(|w| w.fill_color(crate::Rgba8::rgb(1, 2, 3)).expect("fill")),
    ];
    for bytes in base_state_leaks {
        assert_eq!(
            validate_layers(&bytes),
            Err(LayerError::UnisolatedState {
                layer: LayerId::Background
            })
        );
    }
}

#[test]
fn unknown_opcodes_skip_inside_layers_but_unknown_layer_ids_fail() {
    // An unknown opcode with sound framing inside a layer is a counted
    // skip (version policy).
    let mut inside = scene(|w| {
        w.begin_layer(LayerId::Background).expect("begin");
        w.end_layer(LayerId::Background).expect("end");
    });
    // Splice an unknown 0x7f command inside the state-isolation envelope.
    inside.splice(8..8, [0x7f, 0, 0]);
    let report = validate_layers(&inside).expect("skips unknown opcode");
    assert_eq!(report.unknown, 1);
    assert_eq!(report.commands[0], 3);

    // The same unknown opcode outside any layer cannot be placed.
    let outside = {
        let mut bytes = scene(|_| {});
        bytes.extend_from_slice(&[0x7f, 0, 0]);
        bytes
    };
    assert_eq!(
        validate_layers(&outside),
        Err(LayerError::CommandOutsideLayer)
    );

    // An unknown layer id fails the frame at decode: its criticality
    // cannot be placed, so nothing may be painted.
    let mut bad_id = scene(|w| {
        simple_layer(w, LayerId::Background);
    });
    bad_id[4] = 9; // BEGIN_LAYER payload byte
    assert_eq!(
        validate_layers(&bad_id),
        Err(LayerError::Decode(DecodeError::BadPayload { opcode: 0x50 }))
    );

    for (opcode, payload) in [
        (0x50, &[][..]),
        (0x50, &[0, 0][..]),
        (0x51, &[][..]),
        (0x51, &[0, 0][..]),
    ] {
        assert_eq!(
            validate_layers(&single_raw_command(opcode, payload)),
            Err(LayerError::Decode(DecodeError::BadPayload { opcode }))
        );
    }
}

#[test]
fn truncation_at_every_byte_boundary_is_never_silently_complete() {
    let bytes = scene(|w| {
        simple_layer(w, LayerId::Background);
        w.begin_layer(LayerId::Attitude).expect("begin");
        w.rect(PaintMode::Fill, 0.0, 0.0, 2.0, 2.0).expect("rect");
        w.end_layer(LayerId::Attitude).expect("end");
    });
    let full = validate_layers(&bytes).expect("full scene validates");
    for len in 0..bytes.len() {
        // A prefix ending exactly at a layer boundary is a valid
        // *smaller* scene; it must never report the full content.
        // Detecting a missing required layer is the consumer's check
        // via `LayerReport::contains`. Every other prefix must error.
        if let Ok(report) = validate_layers(&bytes[..len]) {
            assert_ne!(
                report.present, full.present,
                "truncation to {len} bytes reported the full layer set"
            );
        }
    }
}

#[test]
fn budgets_are_enforced() {
    // Exactly at the per-layer command budget: legal.
    let at_budget = scene(|w| {
        w.begin_layer(LayerId::Background).expect("begin");
        for _ in 0..(MAX_LAYER_COMMANDS - 2) / 2 {
            w.save().expect("save");
            w.restore().expect("restore");
        }
        w.end_layer(LayerId::Background).expect("end");
    });
    let report = validate_layers(&at_budget).expect("at budget validates");
    assert_eq!(usize::from(report.commands[0]), MAX_LAYER_COMMANDS);

    // One command past the budget: the frame fails.
    let over_budget = scene(|w| {
        w.begin_layer(LayerId::Background).expect("begin");
        for _ in 0..(MAX_LAYER_COMMANDS - 2) / 2 {
            w.save().expect("save");
            w.restore().expect("restore");
        }
        w.rotate(0.1).expect("rotate");
        w.end_layer(LayerId::Background).expect("end");
    });
    assert_eq!(
        validate_layers(&over_budget),
        Err(LayerError::OverCapacity {
            layer: LayerId::Background
        })
    );

    let oversized = std::vec![0u8; MAX_SCENE_BYTES + 1];
    assert_eq!(
        validate_layers(&oversized),
        Err(LayerError::SceneTooLarge {
            bytes: MAX_SCENE_BYTES + 1
        })
    );
}

#[test]
fn graphics_state_stack_budget_is_enforced() {
    let at_budget = scene(|w| {
        w.begin_layer(LayerId::Background).expect("begin");
        for _ in 1..MAX_STACK_DEPTH {
            w.save().expect("save");
        }
        for _ in 1..MAX_STACK_DEPTH {
            w.restore().expect("restore");
        }
        w.end_layer(LayerId::Background).expect("end");
    });
    assert!(validate_layers(&at_budget).is_ok());

    let over_budget = scene(|w| {
        w.begin_layer(LayerId::Background).expect("begin");
        for _ in 0..MAX_STACK_DEPTH {
            w.save().expect("save");
        }
    });
    assert_eq!(
        validate_layers(&over_budget),
        Err(LayerError::StackOverCapacity {
            layer: LayerId::Background,
            depth: MAX_STACK_DEPTH + 1,
        })
    );
}

#[test]
fn layer_marker_corpus_pins_wire_and_legacy_state_isolation() {
    let bytes = scene(|w| {
        w.begin_layer(LayerId::Background).expect("begin");
        w.end_layer(LayerId::Background).expect("end");
    });
    assert_eq!(
        bytes,
        std::vec![
            crate::SCENE_FORMAT_VERSION,
            0x50,
            1,
            0,
            0,
            0x01,
            0,
            0,
            0x02,
            0,
            0,
            0x51,
            1,
            0,
            0,
        ]
    );
    assert!(validate_layers(&bytes).is_ok());
}

#[test]
fn layer_decode_error_preserves_its_source() {
    let error = LayerError::from(DecodeError::Truncated);
    assert!(std::error::Error::source(&error).is_some());
}

#[test]
fn ranges_slice_out_exact_layer_content() {
    let bytes = scene(|w| {
        w.begin_layer(LayerId::Background).expect("begin");
        w.fill_color(crate::Rgba8::rgb(1, 2, 3)).expect("fill");
        w.end_layer(LayerId::Background).expect("end");
        w.begin_layer(LayerId::Attitude).expect("begin");
        w.line(0.0, 0.0, 1.0, 1.0).expect("line");
        w.end_layer(LayerId::Attitude).expect("end");
    });
    let report = validate_layers(&bytes).expect("validates");

    // The attitude range includes the mandatory compatibility envelope.
    let isolated = scene(|w| {
        w.save().expect("save");
        w.line(0.0, 0.0, 1.0, 1.0).expect("line");
        w.restore().expect("restore");
    });
    let (start, end) = report.ranges[LayerId::Attitude.to_u8() as usize].expect("range");
    assert_eq!(&bytes[start..end], &isolated[1..], "content bytes match");
    assert!(report.ranges[LayerId::Tapes.to_u8() as usize].is_none());
}

proptest! {
    // Any ascending subset of layers filled with balanced simple command
    // runs is a legal frame with the reported content.
    #[test]
    fn legal_layered_scenes_always_validate(
        mask in 0u8..(1 << LAYER_COUNT),
        fills in proptest::collection::vec(0usize..40, LAYER_COUNT),
    ) {
        let bytes = scene(|w| {
            for layer in ALL {
                if mask & (1 << layer.to_u8()) == 0 {
                    continue;
                }
                w.begin_layer(layer).expect("begin");
                for i in 0..fills[layer.to_u8() as usize] {
                    match i % 3 {
                        0 => w.line(0.0, 0.0, i as f32, 1.0).expect("line"),
                        1 => w.rect(PaintMode::Fill, 0.0, 0.0, 1.0, 1.0).expect("rect"),
                        _ => {
                            w.save().expect("save");
                            w.restore().expect("restore");
                        }
                    }
                }
                w.end_layer(layer).expect("end");
            }
        });
        let report = validate_layers(&bytes).expect("legal scene validates");
        prop_assert_eq!(report.present, mask);
    }
}
