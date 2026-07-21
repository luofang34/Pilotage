//! Deterministic device → profile selection (ADR-0007).
//!
//! A connected device is matched to a profile by its USB vendor/product
//! identity, with an explicit generic fallback so an unknown device still maps
//! rather than silently producing no control. The selection is a pure function
//! of the candidate list, never a clock or a registry mutation, so the same
//! device always resolves to the same profile — and a candidate list in which
//! more than one profile claims the same identity is rejected outright, never
//! resolved by list order.

use crate::profile::{DeviceIdentity, DeviceProfile};

impl DeviceIdentity {
    /// The wildcard identity a generic fallback profile declares: it matches no
    /// specific device, so it is selected only when no vendor profile does.
    pub const WILDCARD: Self = Self {
        vendor_id: 0,
        product_id: 0,
    };
}

/// Why device → profile selection failed. Ambiguity is rejected, not
/// order-resolved: two profiles claiming one identity would make the selected
/// mapping depend on registry iteration order, which is exactly the silent
/// drift ADR-0007 exists to prevent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SelectError {
    /// More than one profile claims the same exact vendor/product identity.
    #[error(
        "{count} profiles claim device {vendor_id:04x}:{product_id:04x}; \
         selection would depend on candidate order"
    )]
    AmbiguousExact {
        /// USB vendor ID of the contested identity.
        vendor_id: u16,
        /// USB product ID of the contested identity.
        product_id: u16,
        /// How many candidates claim it.
        count: usize,
    },
    /// More than one generic (wildcard) fallback profile is present.
    #[error("{count} generic fallback profiles present; selection would depend on candidate order")]
    AmbiguousWildcard {
        /// How many wildcard candidates are present.
        count: usize,
    },
}

/// Selects the profile for `identity` from `candidates`: the unique exact
/// vendor/product match, else the unique [`DeviceIdentity::WILDCARD`]
/// (generic) profile, else `Ok(None)`.
///
/// The two passes make the fallback explicit and visible — an unknown device
/// resolves to the generic profile, never to a vendor profile that merely
/// happened to sort first — and never pick the wildcard over a real match.
/// A wildcard `identity` (a device whose vendor/product could not be read)
/// takes the fallback path directly, so it can never "exactly match" the
/// generic profile and mask a real ambiguity.
///
/// # Errors
///
/// Returns [`SelectError`] when more than one candidate claims the matched
/// identity (exact or wildcard) — an ambiguous registry fails closed instead
/// of resolving by order.
pub fn select_by_identity(
    identity: DeviceIdentity,
    candidates: &[DeviceProfile],
) -> Result<Option<&DeviceProfile>, SelectError> {
    if identity != DeviceIdentity::WILDCARD {
        let mut exact = candidates
            .iter()
            .filter(|profile| profile.identity() == identity);
        if let Some(first) = exact.next() {
            let extras = exact.count();
            if extras > 0 {
                return Err(SelectError::AmbiguousExact {
                    vendor_id: identity.vendor_id,
                    product_id: identity.product_id,
                    count: extras + 1,
                });
            }
            return Ok(Some(first));
        }
    }
    let mut wildcards = candidates
        .iter()
        .filter(|profile| profile.identity() == DeviceIdentity::WILDCARD);
    let Some(first) = wildcards.next() else {
        return Ok(None);
    };
    let extras = wildcards.count();
    if extras > 0 {
        return Err(SelectError::AmbiguousWildcard { count: extras + 1 });
    }
    Ok(Some(first))
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic)]
mod tests {
    use super::{DeviceIdentity, SelectError, select_by_identity};
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
            .expect("unambiguous")
            .expect("the exact device is present");
        assert_eq!(selected.identity(), identity(0x1209, 0x4f54));
    }

    #[test]
    fn an_unknown_device_falls_back_to_the_wildcard() {
        let candidates = [profile(0x1209, 0x4f54), profile(0, 0)];
        let selected = select_by_identity(identity(0xdead, 0xbeef), &candidates)
            .expect("unambiguous")
            .expect("wildcard is present");
        assert_eq!(selected.identity(), DeviceIdentity::WILDCARD);
    }

    #[test]
    fn no_match_and_no_wildcard_is_none() {
        let candidates = [profile(0x1209, 0x4f54)];
        assert_eq!(
            select_by_identity(identity(0xdead, 0xbeef), &candidates).expect("unambiguous"),
            None
        );
    }

    #[test]
    fn two_profiles_claiming_one_device_are_rejected() {
        let candidates = [
            profile(0x1209, 0x4f54),
            profile(0, 0),
            profile(0x1209, 0x4f54),
        ];
        assert_eq!(
            select_by_identity(identity(0x1209, 0x4f54), &candidates),
            Err(SelectError::AmbiguousExact {
                vendor_id: 0x1209,
                product_id: 0x4f54,
                count: 2,
            })
        );
    }

    #[test]
    fn two_wildcards_are_rejected_for_an_unknown_device() {
        let candidates = [profile(0, 0), profile(0x1209, 0x4f54), profile(0, 0)];
        assert_eq!(
            select_by_identity(identity(0xdead, 0xbeef), &candidates),
            Err(SelectError::AmbiguousWildcard { count: 2 })
        );
    }

    #[test]
    fn a_wildcard_identity_device_takes_the_fallback_path() {
        // A device whose vendor/product could not be read resolves through the
        // SAME wildcard rules as any unknown device — including the ambiguity
        // rejection — rather than "exactly matching" a generic profile.
        let unambiguous = [profile(0x1209, 0x4f54), profile(0, 0)];
        let selected = select_by_identity(DeviceIdentity::WILDCARD, &unambiguous)
            .expect("unambiguous")
            .expect("wildcard present");
        assert_eq!(selected.identity(), DeviceIdentity::WILDCARD);

        let ambiguous = [profile(0, 0), profile(0, 0)];
        assert_eq!(
            select_by_identity(DeviceIdentity::WILDCARD, &ambiguous),
            Err(SelectError::AmbiguousWildcard { count: 2 })
        );
    }
}
