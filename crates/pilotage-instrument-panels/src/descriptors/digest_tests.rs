#![allow(clippy::expect_used, clippy::panic)]

use std::string::String;
use std::vec;

use pilotage_instrument_registry::{PanelDescriptor, Registry, scene_digest};
use pilotage_instrument_scene::MAX_SCENE_BYTES;

use super::{BUILTIN_PANELS, HSI_DESCRIPTOR, PFD_DESCRIPTOR};

fn hex(digest: [u8; 32]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(64);
    for byte in digest {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

fn digest_of(panels: &'static [PanelDescriptor]) -> String {
    let registry = Registry::new(panels).expect("composes");
    let mut scratch = vec![0u8; MAX_SCENE_BYTES];
    hex(scene_digest(&registry, &mut scratch).expect("digests"))
}

#[test]
fn the_builtin_scene_digest_is_pinned() {
    assert_eq!(digest_of(BUILTIN_PANELS), super::BUILTIN_SCENE_DIGEST);
}

#[test]
fn the_digest_moves_when_the_composition_moves() {
    static PFD_ONLY: [PanelDescriptor; 1] = [PFD_DESCRIPTOR];
    static REVERSED: [PanelDescriptor; 2] = [HSI_DESCRIPTOR, PFD_DESCRIPTOR];
    let full = digest_of(BUILTIN_PANELS);
    assert_ne!(digest_of(&PFD_ONLY), full, "dropping a panel must move it");
    assert_ne!(digest_of(&REVERSED), full, "panel order is contractual");
}
