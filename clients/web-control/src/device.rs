//! The browser device stage: raw Gamepad/keyboard input → the canonical pad
//! layout the control scheme binds, driven entirely by shared-engine device
//! profiles (INPUT-01).
//!
//! Every mapping decision here is DATA: which physical axis feeds which
//! canonical slot, which key drives which slot or button, and which profile a
//! connected pad resolves to. Resolution runs through the shared layered
//! registry — per-layer `select_by_identity` (so the browser and a native
//! host make identical selections, including the fail-closed ambiguity
//! rejection) followed by `merge_layers` across the built-in < organization
//! < user < vehicle < session precedence chain. No key list, axis index, or
//! controller table lives in shell JavaScript — the shell passes
//! `Gamepad.id` strings, raw samples, and key events through verbatim.

use pilotage_input::{
    DeviceIdentity, DeviceProfile, LayeredProfile, ProfileLayer, layered, merge_layers,
    parse_profile_bytes, select_by_identity,
};

use crate::sample::{ButtonSample, RawSample};

mod compiled;
pub(crate) use compiled::CompiledDevice;

/// Canonical pad slot counts — the fixed geometry the scheme binds against.
pub const MAX_AXES: usize = 8;
/// Canonical button slot count.
pub const MAX_BUTTONS: usize = 24;

/// Built-in browser device profiles, embedded like the default scheme so a
/// fresh viewer maps devices with no privileged path or network fetch.
const KEYBOARD_JSON: &[u8] = include_bytes!("profiles/devices/keyboard.json");
/// The keyboard's reserved registry identity (`keyboard.json` declares it):
/// keyboard resolution runs through the SAME layered registry as pads —
/// select per layer, merge across layers — so an org/user/session layer can
/// override key bindings, and the keyboard's own revision and digest are
/// announced when it is the active source.
pub const KEYBOARD_IDENTITY: DeviceIdentity = DeviceIdentity {
    vendor_id: 0,
    product_id: 1,
};
const GENERIC_PAD_JSON: &[u8] = include_bytes!("profiles/devices/generic-pad.json");
const DUALSENSE_JSON: &[u8] = include_bytes!("profiles/devices/dualsense.json");
const RADIOMASTER_POCKET_JSON: &[u8] = include_bytes!("profiles/devices/radiomaster-pocket.json");

/// How a pad selection resolved, mirrored to the shell as a plain code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectOutcome {
    /// No usable profile: a layer was ambiguous for this identity, no layer
    /// matched, or the merged profile failed to compile. The stage keeps NO
    /// pad map — fail closed.
    Refused,
    /// An exact vendor/product profile matched (in at least one layer).
    Exact,
    /// Only wildcard profiles contributed — the generic fallback path.
    Fallback,
}

/// The stateful device stage: the selected pad map, the keyboard map, the
/// held-key set, and the layered profile registry.
#[derive(Debug, Default)]
pub struct DeviceStage {
    pad: Option<CompiledDevice>,
    pad_outcome: Option<SelectOutcome>,
    keyboard: Option<CompiledDevice>,
    layers: Vec<LayeredProfile<DeviceProfile>>,
    held: Vec<String>,
}

impl DeviceStage {
    /// Builds the stage from the embedded profile set (all in the built-in
    /// layer). A built-in that fails to parse leaves its map empty (that
    /// input source emits nothing) — the Rust test suite proves the embedded
    /// set compiles, so this is a build-integrity backstop, not an expected
    /// path.
    pub fn new() -> Self {
        let mut stage = Self {
            layers: [
                KEYBOARD_JSON,
                GENERIC_PAD_JSON,
                DUALSENSE_JSON,
                RADIOMASTER_POCKET_JSON,
            ]
            .iter()
            .filter_map(|bytes| parse_profile_bytes(bytes).ok())
            .map(|profile| layered(ProfileLayer::BuiltIn, profile))
            .collect(),
            ..Self::default()
        };
        stage.keyboard = stage.resolve_keyboard();
        // A pad connected before any identity arrives maps like today's raw
        // pass-through: the wildcard standard-mapping profile.
        stage.select_pad("");
        stage
    }

    /// Resolves the keyboard through the layered registry by its reserved
    /// identity — the same per-layer selection and cross-layer merge a pad
    /// takes, so overrides land through one path.
    pub(crate) fn resolve_keyboard(&self) -> Option<CompiledDevice> {
        let (device, _) = self.resolve_identity(KEYBOARD_IDENTITY, false);
        device
    }

    /// Installs a freshly resolved keyboard map (a registry-layer change);
    /// the coordinator calls this only at a transaction boundary.
    pub(crate) fn install_keyboard(&mut self, keyboard: Option<CompiledDevice>) {
        self.keyboard = keyboard;
    }

    /// The keyboard profile's human-readable label (empty if the embedded
    /// set failed to compile — a build-integrity backstop).
    #[must_use]
    pub fn keyboard_label(&self) -> &str {
        self.keyboard.as_ref().map_or("", |device| device.label())
    }

    /// The keyboard profile's document revision.
    #[must_use]
    pub fn keyboard_revision(&self) -> u32 {
        self.keyboard.as_ref().map_or(0, CompiledDevice::revision)
    }

    /// The keyboard profile's effective-content digest.
    #[must_use]
    pub fn keyboard_digest(&self) -> Option<[u8; 32]> {
        self.keyboard.as_ref().map(CompiledDevice::digest)
    }

    /// Adds a profile to `layer`. The current selection is NOT re-run here —
    /// the caller re-resolves so the change takes the same transactional
    /// handover a physical pad swap takes.
    pub fn add_profile(&mut self, layer: ProfileLayer, bytes: &[u8]) -> bool {
        match parse_profile_bytes(bytes) {
            Ok(profile) => {
                self.layers.push(layered(layer, profile));
                true
            }
            Err(_) => false,
        }
    }

    /// Resolves a `Gamepad.id` string through the layered registry WITHOUT
    /// installing the result: each layer selects independently through the
    /// shared `select_by_identity` (an ambiguity within any layer refuses
    /// the pad outright — no map, no control — rather than guessing), then
    /// the per-layer winners merge in precedence order into the effective
    /// profile.
    pub(crate) fn resolve_pad(&self, gamepad_id: &str) -> (Option<CompiledDevice>, SelectOutcome) {
        self.resolve_identity(parse_gamepad_identity(gamepad_id), true)
    }

    /// Layered resolution for any registry identity. `allow_wildcard`
    /// admits the generic fallback path (pads); the keyboard resolves
    /// EXACTLY — a wildcard pad override must never bleed into key
    /// bindings.
    fn resolve_identity(
        &self,
        identity: DeviceIdentity,
        allow_wildcard: bool,
    ) -> (Option<CompiledDevice>, SelectOutcome) {
        let mut contributions: Vec<LayeredProfile<DeviceProfile>> = Vec::new();
        let mut exact = false;
        for layer in [
            ProfileLayer::BuiltIn,
            ProfileLayer::Organization,
            ProfileLayer::User,
            ProfileLayer::Vehicle,
            ProfileLayer::Session,
        ] {
            let candidates: Vec<DeviceProfile> = self
                .layers
                .iter()
                .filter(|entry| entry.layer == layer)
                .filter(|entry| allow_wildcard || entry.profile.identity() == identity)
                .map(|entry| entry.profile.clone())
                .collect();
            if candidates.is_empty() {
                continue;
            }
            match select_by_identity(identity, &candidates) {
                Err(_) => return (None, SelectOutcome::Refused),
                Ok(None) => {}
                Ok(Some(profile)) => {
                    exact = exact || profile.identity() == identity;
                    contributions.push(layered(layer, profile.clone()));
                }
            }
        }
        let Some(merged) = merge_layers(contributions) else {
            return (None, SelectOutcome::Refused);
        };
        match CompiledDevice::from_profile(&merged) {
            Ok(device) => {
                let outcome = if exact {
                    SelectOutcome::Exact
                } else {
                    SelectOutcome::Fallback
                };
                (Some(device), outcome)
            }
            Err(_) => (None, SelectOutcome::Refused),
        }
    }

    /// Installs a resolved pad map. The coordinator calls this only at a
    /// transaction boundary (or when nothing changed), never mid-hold.
    pub(crate) fn install_pad(&mut self, pad: Option<CompiledDevice>, outcome: SelectOutcome) {
        self.pad = pad;
        self.pad_outcome = Some(outcome);
    }

    /// Resolves and installs in one step — the pre-transactional seam kept
    /// for stage-level tests; production selection goes through the
    /// coordinator's handover.
    pub fn select_pad(&mut self, gamepad_id: &str) -> SelectOutcome {
        let (pad, outcome) = self.resolve_pad(gamepad_id);
        self.install_pad(pad, outcome);
        outcome
    }

    /// The selected pad profile's human-readable label, empty when refused.
    pub fn pad_label(&self) -> &str {
        self.pad.as_ref().map_or("", |pad| pad.label())
    }

    /// The selected pad profile's document revision, zero when refused.
    #[must_use]
    pub fn pad_revision(&self) -> u32 {
        self.pad.as_ref().map_or(0, CompiledDevice::revision)
    }

    /// The selected pad profile's effective-content digest, or `None` when
    /// no pad map is installed.
    #[must_use]
    pub fn pad_digest(&self) -> Option<[u8; 32]> {
        self.pad.as_ref().map(CompiledDevice::digest)
    }

    /// Records one key transition. Keys arrive canonical from the shell
    /// (letters lower-cased); the profile data speaks the same convention.
    pub fn key_event(&mut self, key: &str, pressed: bool) {
        if pressed {
            if !self.held.iter().any(|held| held == key) {
                self.held.push(key.to_owned());
            }
        } else {
            self.held.retain(|held| held != key);
        }
    }

    /// Drops every held key (window blur, session teardown): a key released
    /// while the page was blurred must not remain a phantom hold.
    pub fn clear_keys(&mut self) {
        self.held.clear();
    }

    /// Whether the keyboard profile binds `key` — the shell's only key
    /// question (which keys to capture from the page), answered from profile
    /// data so no key list exists outside it.
    #[must_use]
    pub fn key_is_bound(&self, key: &str) -> bool {
        self.keyboard
            .as_ref()
            .is_some_and(|keyboard| keyboard.binds_key(key))
    }

    /// The keyboard map's key bound to a canonical button slot, if any.
    #[must_use]
    pub fn keyboard_key_for_button(&self, slot: usize) -> Option<&str> {
        self.keyboard.as_ref()?.key_for_button(slot)
    }

    /// The installed pad map's printed name for a canonical button slot.
    #[must_use]
    pub fn pad_button_label(&self, slot: usize) -> Option<&str> {
        self.pad.as_ref()?.button_label(slot)
    }

    /// Translates a raw pad sample through the selected pad map into `out`,
    /// returning the canonical axis/button counts. With no usable map
    /// (refused selection) the sample is empty and nothing can drive control.
    pub fn pad_sample(
        &self,
        axes: &[f32],
        buttons: &[ButtonSample],
        out: &mut RawSample,
    ) -> (usize, usize) {
        let Some(pad) = &self.pad else {
            out.axes.clear();
            out.buttons.clear();
            return (0, 0);
        };
        pad.translate_pad(axes, buttons, out);
        (out.axes.len(), out.buttons.len())
    }

    /// Synthesizes the canonical sample from held keys via the keyboard
    /// profile, returning the canonical axis/button counts.
    pub fn key_sample(&self, out: &mut RawSample) -> (usize, usize) {
        let Some(keyboard) = &self.keyboard else {
            out.axes.clear();
            out.buttons.clear();
            return (0, 0);
        };
        keyboard.synthesize_keys(&self.held, out);
        (out.axes.len(), out.buttons.len())
    }
}

/// Extracts the USB vendor/product identity from a browser `Gamepad.id`.
///
/// Chromium encodes `"... (Vendor: 054c Product: 0ce6)"`; Firefox encodes
/// `"054c-0ce6-DualSense Wireless Controller"`. Anything unparsable maps to
/// the wildcard identity, which resolves through the generic fallback path.
pub fn parse_gamepad_identity(id: &str) -> DeviceIdentity {
    if let Some(identity) = parse_chromium_identity(id) {
        return identity;
    }
    if let Some(identity) = parse_firefox_identity(id) {
        return identity;
    }
    if let Some(identity) = parse_named_identity(id) {
        return identity;
    }
    DeviceIdentity::WILDCARD
}

/// Controllers a client can only name.
///
/// The browser hands over a string carrying the USB identity, which is what
/// this registry is keyed on. Apple's GameController framework exposes a
/// product NAME and no vendor/product pair at all, so a client built on it has
/// nothing the key can match and every pad would fall to the wildcard — the
/// generic mapping, whichever pad the reader is actually holding.
///
/// Only pads whose profile routes each source axis straight through may be
/// listed, and that is a constraint about the caller rather than the pads.
///
/// A profile's `source_index` numbers the axes AS THE BROWSER DELIVERS THEM.
/// The client that has no USB pair to offer is Apple's, and it does not
/// deliver browser axes: it reads `leftThumbstick`/`rightThumbstick`, which
/// name the sticks semantically, so what arrives is already in canonical slot
/// order whatever the pad reports underneath. Handing those to a profile that
/// reroutes applies a correction to input that never needed it — the sticks
/// come out on the wrong controls and the inverted ones come out backwards.
///
/// So a rerouting pad is deliberately absent here and falls to the wildcard,
/// whose straight-through packing is the correct reading of a canonical
/// stick. Recognising it by name would name it right and fly it wrong.
/// `named_pads_route_straight_through` holds this.
const NAMED_IDENTITIES: [(&str, DeviceIdentity); 1] = [(
    "dualsense",
    DeviceIdentity {
        vendor_id: 0x054c,
        product_id: 0x0ce6,
    },
)];

/// Matches a product name against the pads named above.
///
/// Substring rather than equality: the same pad is reported as "DualSense",
/// "DualSense Wireless Controller" and "Sony DualSense" by different layers,
/// and all three name one device.
fn parse_named_identity(id: &str) -> Option<DeviceIdentity> {
    let lowered = id.to_ascii_lowercase();
    NAMED_IDENTITIES
        .iter()
        .find(|(name, _)| lowered.contains(name))
        .map(|(_, identity)| *identity)
}

fn parse_chromium_identity(id: &str) -> Option<DeviceIdentity> {
    let vendor = hex_after(id, "Vendor:")?;
    let product = hex_after(id, "Product:")?;
    Some(DeviceIdentity {
        vendor_id: vendor,
        product_id: product,
    })
}

fn parse_firefox_identity(id: &str) -> Option<DeviceIdentity> {
    let mut parts = id.splitn(3, '-');
    let vendor = u16::from_str_radix(parts.next()?.trim(), 16).ok()?;
    let product = u16::from_str_radix(parts.next()?.trim(), 16).ok()?;
    parts.next()?;
    Some(DeviceIdentity {
        vendor_id: vendor,
        product_id: product,
    })
}

fn hex_after(haystack: &str, marker: &str) -> Option<u16> {
    let rest = haystack.split(marker).nth(1)?.trim_start();
    let token: String = rest.chars().take_while(char::is_ascii_hexdigit).collect();
    u16::from_str_radix(&token, 16).ok()
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod apple_identity_tests {
    use super::{DeviceIdentity, parse_gamepad_identity};

    #[test]
    fn a_pad_can_be_recognised_by_name_when_no_usb_identity_is_offered() {
        // The browser hands over a string carrying the USB identity, and the
        // registry keys on that. The Apple client has no such string to give:
        // GameController reports a product NAME and no vendor/product pair.
        // Without a name lookup every pad on an iPad falls to the wildcard and
        // the reader gets the generic packing whichever pad they hold.
        // A pad this repo carries a profile for is now recognised by name, so
        // the reader gets the law written for their pad rather than the
        // generic packing.
        assert_eq!(
            parse_gamepad_identity("DualSense Wireless Controller"),
            DeviceIdentity {
                vendor_id: 0x054c,
                product_id: 0x0ce6
            },
        );
        assert_eq!(
            parse_gamepad_identity("Sony DualSense"),
            DeviceIdentity {
                vendor_id: 0x054c,
                product_id: 0x0ce6
            },
            "the same pad under another of its names"
        );

        // A pad nobody wrote a profile for still falls to the wildcard, which
        // is the honest answer rather than a guess at its packing.
        //
        // The RadioMaster is in that list on purpose. This repo does carry a
        // profile for it, but that profile reroutes browser axis order, and
        // the only caller that reaches this path sends canonical stick order
        // already. Its name is withheld so it gets the straight-through
        // packing that reading is entitled to.
        for unknown in [
            "Xbox Wireless Controller",
            "RadioMaster Pocket",
            "gamepad",
            "",
        ] {
            assert_eq!(
                parse_gamepad_identity(unknown),
                DeviceIdentity::WILDCARD,
                "{unknown} resolved to an identity nobody declared"
            );
        }

        // The same pad, as the browser names it, does carry one.
        let chromium =
            "DualSense Wireless Controller (STANDARD GAMEPAD Vendor: 054c Product: 0ce6)";
        assert_eq!(
            parse_gamepad_identity(chromium),
            DeviceIdentity {
                vendor_id: 0x054c,
                product_id: 0x0ce6
            },
            "the browser's form carries the identity the registry needs"
        );
    }
}
