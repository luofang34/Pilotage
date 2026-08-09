//! Proofs for the shipped screen composition: it validates, its digest
//! is stable shape, and a layout can gain the monitor panel with no
//! renderer change — composition data alone drives the new slot.

#![allow(clippy::expect_used, clippy::panic)]

use indicate_instrument_panels::BUILTIN_CRITICALITY_BANDS;
use indicate_instrument_registry::{
    CompositionDescriptor, DesignFrame, Region, Slot, composition_digest, validate_composition,
};

use super::{SCREEN_COMPOSITION, composition, composition_digest_hex, composition_slot_count};
use crate::registry::registry;
use crate::tests::{attitude_state, write_state};
use crate::{RenderStatus, Runtime};

/// The bench arrangement: the shipped two slots plus the monitor on a
/// taller screen. No slot overlaps, so no occlusion is declared.
const WITH_MONITOR: CompositionDescriptor = CompositionDescriptor {
    screen: DesignFrame {
        width: 960.0,
        height: 720.0,
    },
    slots: &[
        Slot {
            panel: "pfd",
            rect: Region {
                x: 0.0,
                y: 0.0,
                width: 480.0,
                height: 360.0,
            },
            occludes: &[],
        },
        Slot {
            panel: "hsi",
            rect: Region {
                x: 480.0,
                y: 0.0,
                width: 480.0,
                height: 360.0,
            },
            occludes: &[],
        },
        Slot {
            panel: "monitor",
            rect: Region {
                x: 0.0,
                y: 360.0,
                width: 480.0,
                height: 360.0,
            },
            occludes: &[],
        },
    ],
};

#[test]
fn shipped_composition_validates_and_enumerates() {
    let composition = composition().expect("shipped composition validates");
    assert_eq!(composition_slot_count(), 2);
    assert_eq!(composition.slots[0].panel, "pfd");
    assert_eq!(composition.slots[1].panel, "hsi");
    let digest = composition_digest_hex();
    assert_eq!(digest.len(), 64, "lowercase hex digest: {digest}");
}

#[test]
fn a_layout_gains_the_monitor_without_renderer_changes() {
    let registry = registry().expect("sets compose");
    // The same registry and the same measured bands admit the taller
    // layout: adding a slot is a declaration, not a code change.
    validate_composition(&registry, &WITH_MONITOR, &BUILTIN_CRITICALITY_BANDS)
        .expect("monitor layout validates against the same registry");

    // The monitor renders through the runtime's existing path: resolve
    // its index from the registry, write an ordinary state frame, and
    // the one render entry point produces a committed scene.
    let (monitor, _) = registry
        .panels()
        .enumerate()
        .find(|(_, panel)| panel.id == "monitor")
        .expect("monitor is registered");
    let mut runtime = Runtime::new();
    write_state(&mut runtime, &attitude_state());
    let outcome = runtime.render(monitor as u32);
    assert_eq!(outcome.status, RenderStatus::Ok);
    assert!(outcome.scene_len > 1, "monitor rendered no scene");
    assert_eq!(outcome.generation, 1);

    // A different layout is a different identity: the monitor
    // composition's digest must differ from the shipped one.
    let mut scratch = vec![0u8; indicate_instrument_scene::MAX_SCENE_BYTES];
    let with_monitor =
        composition_digest(&registry, &WITH_MONITOR, &mut scratch).expect("digest computes");
    let shipped =
        composition_digest(&registry, &SCREEN_COMPOSITION, &mut scratch).expect("digest computes");
    assert_ne!(with_monitor, shipped, "layout change moves the digest");
}
