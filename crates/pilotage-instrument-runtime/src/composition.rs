//! The shipped screen composition (ADR-0032): which panels paint where
//! on the one logical screen, validated at init and digested for the
//! compatibility tuple.
//!
//! A composition is layout and nothing else. Slot index is the paint
//! order. Shells enumerate the slots and pin the digest; they do not
//! hold a panel list of their own.

use indicate_instrument_panels::BUILTIN_CRITICALITY_BANDS;
use indicate_instrument_registry::{
    CompositionDescriptor, DesignFrame, Region, Slot, composition_digest, validate_composition,
};

use crate::registry::registry;

/// The shipped screen composition: the PFD left and the HSI right on
/// one 960×360 logical screen — today's two-panel G5 arrangement, with
/// no overlap and no declared occlusion.
///
/// Video, maps, and SVS stay outside this deterministic panel
/// composition. Their contracts differ: they are not Indicate panel
/// scenes, so the composition floor does not reach them (ADR-0032).
pub const SCREEN_COMPOSITION: CompositionDescriptor = CompositionDescriptor {
    screen: DesignFrame {
        width: 960.0,
        height: 360.0,
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
    ],
};

/// The validated shipped composition, or `None` if it no longer
/// validates against the composed registry and the measured criticality
/// bands — fail-closed, like the registry. Validated once and
/// memoized: both inputs are `&'static` and validation is pure, so the
/// answer cannot change between calls.
pub fn composition() -> Option<&'static CompositionDescriptor> {
    static VALIDATED: std::sync::OnceLock<Option<&'static CompositionDescriptor>> =
        std::sync::OnceLock::new();
    *VALIDATED.get_or_init(|| {
        let registry = registry()?;
        validate_composition(&registry, &SCREEN_COMPOSITION, &BUILTIN_CRITICALITY_BANDS).ok()?;
        Some(&SCREEN_COMPOSITION)
    })
}

/// Number of slots in the shipped composition, or zero when it does
/// not validate.
pub fn composition_slot_count() -> u32 {
    composition().map_or(0, |composition| composition.slots.len() as u32)
}

fn slot(index: u32) -> Option<&'static Slot> {
    composition()?.slots.get(index as usize)
}

/// The panel id a slot paints, or the empty string for an unknown
/// slot.
pub fn composition_slot_panel(index: u32) -> String {
    slot(index).map_or_else(String::new, |slot| slot.panel.to_string())
}

/// The rectangle a slot paints at, in screen units, or `None` for an
/// unknown slot.
pub fn composition_slot_rect(index: u32) -> Option<Region> {
    slot(index).map(|slot| slot.rect)
}

/// The screen-composition digest over the composed registry, as
/// lowercase hex — the fifth compatibility-tuple value (ADR-0032).
/// Shells pin it against their own literal so a layout change is a
/// deliberate re-pin, never drift. An empty string matches no pin: a
/// digest failure fails visibly.
pub fn composition_digest_hex() -> String {
    let Some(registry) = registry() else {
        return String::new();
    };
    let mut scratch = vec![0u8; indicate_instrument_scene::MAX_SCENE_BYTES];
    match composition_digest(&registry, &SCREEN_COMPOSITION, &mut scratch) {
        Ok(digest) => {
            const HEX: &[u8; 16] = b"0123456789abcdef";
            let mut out = String::with_capacity(64);
            for byte in digest {
                out.push(HEX[(byte >> 4) as usize] as char);
                out.push(HEX[(byte & 0x0f) as usize] as char);
            }
            out
        }
        Err(_) => String::new(),
    }
}

#[cfg(test)]
mod tests;
