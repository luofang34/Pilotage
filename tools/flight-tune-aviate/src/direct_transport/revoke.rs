//! One idempotent removal of every direct authority.
//!
//! Revoke is not release. A release sends the frozen baseline and keeps
//! direct mode through the observation window. Revoke removes the
//! authority itself: after it, the transport prepares nothing, enacts
//! nothing, and freezes nothing, and a command prepared before the revoke
//! can no longer be enacted. The production runtime calls it from terminal
//! and recovery paths, where the only safe assumption is that it may
//! already have run.

use flight_tune::Digest;

/// The result of revoking one direct transport's authority.
///
/// The receipt is stable across calls: a second revoke returns the same
/// value, and reports that it removed nothing.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DirectRevokeReceipt {
    transport_identity_digest: Digest,
    removed_authority: bool,
    released_baseline: bool,
}

impl DirectRevokeReceipt {
    pub(super) const fn new(
        transport_identity_digest: Digest,
        removed_authority: bool,
        released_baseline: bool,
    ) -> Self {
        Self {
            transport_identity_digest,
            removed_authority,
            released_baseline,
        }
    }

    /// The transport whose authority the revoke removed.
    #[must_use]
    pub const fn transport_identity_digest(&self) -> Digest {
        self.transport_identity_digest
    }

    /// Whether this call was the one that removed the authority.
    #[must_use]
    pub const fn removed_authority(&self) -> bool {
        self.removed_authority
    }

    /// Whether this call was the one that released a frozen baseline.
    #[must_use]
    pub const fn released_baseline(&self) -> bool {
        self.released_baseline
    }
}
