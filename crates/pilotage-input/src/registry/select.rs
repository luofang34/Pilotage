//! Deterministic device → profile selection (ADR-0007).
//!
//! A connected device is matched to a profile by its USB vendor/product
//! identity, with an explicit generic fallback so an unknown device still maps
//! rather than silently producing no control. The selection is a pure function
//! of the candidate list, never a clock or a registry mutation, so the same
//! device always resolves to the same profile.

use crate::profile::{DeviceIdentity, DeviceProfile};

impl DeviceIdentity {
    /// The wildcard identity a generic fallback profile declares: it matches no
    /// specific device, so it is selected only when no vendor profile does.
    pub const WILDCARD: Self = Self {
        vendor_id: 0,
        product_id: 0,
    };
}

/// Selects the profile for `identity` from `candidates`: the first exact
/// vendor/product match, else the first [`DeviceIdentity::WILDCARD`] (generic)
/// profile, else `None`.
///
/// The two passes make the fallback explicit and visible — an unknown device
/// resolves to the generic profile, never to a vendor profile that merely
/// happened to sort first — and never picks the wildcard over a real match.
#[must_use]
pub fn select_by_identity(
    identity: DeviceIdentity,
    candidates: &[DeviceProfile],
) -> Option<&DeviceProfile> {
    candidates
        .iter()
        .find(|profile| profile.identity() == identity)
        .or_else(|| {
            candidates
                .iter()
                .find(|profile| profile.identity() == DeviceIdentity::WILDCARD)
        })
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic)]
mod tests {
    use super::{DeviceIdentity, select_by_identity};
    use crate::profile::{DeviceInfo, DeviceProfile};

    fn profile(vendor_id: u16, product_id: u16) -> DeviceProfile {
        DeviceProfile {
            schema_version: 1,
            revision: 1,
            device: DeviceInfo {
                vendor_id,
                product_id,
                product: None,
            },
            description: None,
            axes: vec![],
            buttons: vec![],
        }
    }

    fn identity(vendor_id: u16, product_id: u16) -> DeviceIdentity {
        DeviceIdentity {
            vendor_id,
            product_id,
        }
    }

    #[test]
    fn an_exact_match_wins_over_the_generic_fallback() {
        let candidates = [profile(0, 0), profile(0x1209, 0x4f54)];
        let selected = select_by_identity(identity(0x1209, 0x4f54), &candidates)
            .expect("the exact device is present");
        assert_eq!(selected.identity(), identity(0x1209, 0x4f54));
    }

    #[test]
    fn an_unknown_device_falls_back_to_the_wildcard() {
        let candidates = [profile(0x1209, 0x4f54), profile(0, 0)];
        let selected =
            select_by_identity(identity(0xdead, 0xbeef), &candidates).expect("wildcard is present");
        assert_eq!(selected.identity(), DeviceIdentity::WILDCARD);
    }

    #[test]
    fn no_match_and_no_wildcard_is_none() {
        let candidates = [profile(0x1209, 0x4f54)];
        assert!(select_by_identity(identity(0xdead, 0xbeef), &candidates).is_none());
    }
}
