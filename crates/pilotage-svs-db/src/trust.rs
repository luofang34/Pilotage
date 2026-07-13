//! The trust root and signature block, and verify-only signature checking.
//!
//! A package carries a [`PackageSignature`]: which trust key signed it and the
//! 64-byte Ed25519 signature over the canonical manifest bytes. A [`TrustRoot`]
//! is the set of public keys this device is configured to trust. Verification
//! is one-way: this crate can *check* a signature against a configured key, but
//! holds no private key and cannot *produce* one. Signing lives with the
//! offline publisher (and, in this crate, only in test fixtures), so the
//! airborne/runtime path can authenticate a package but never forge one.

use core::fmt;

use ed25519_dalek::{Signature, VerifyingKey};

use crate::error::DbError;

/// Identity of a trust key (the key a package claims to be signed by).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TrustKeyId(pub u64);

impl fmt::Display for TrustKeyId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "key:{:016x}", self.0)
    }
}

/// A package's signature: the trust key that signed it and the Ed25519
/// signature bytes over the canonical manifest bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PackageSignature {
    /// The trust key the package claims to be signed by.
    pub key_id: TrustKeyId,
    /// The 64-byte Ed25519 signature.
    pub bytes: [u8; 64],
}

/// One configured trust anchor: a key id and its Ed25519 public key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TrustAnchor {
    /// The key id.
    pub key_id: TrustKeyId,
    /// The 32-byte Ed25519 public key.
    pub public_key: [u8; 32],
}

/// The set of trust anchors this device is configured to accept. A package
/// signed by a key absent from this set is refused, however well-formed its
/// signature is.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TrustRoot {
    anchors: Vec<TrustAnchor>,
}

impl TrustRoot {
    /// A trust root over the given anchors.
    #[must_use]
    pub fn new(anchors: Vec<TrustAnchor>) -> Self {
        Self { anchors }
    }

    /// The public key configured for `key_id`, if any.
    #[must_use]
    pub fn public_key(&self, key_id: TrustKeyId) -> Option<[u8; 32]> {
        self.anchors
            .iter()
            .find(|a| a.key_id == key_id)
            .map(|a| a.public_key)
    }
}

/// Verifies `signature` over `message` against `trust`, failing closed: an
/// untrusted key id is [`DbError::UntrustedRoot`], and a malformed key or a
/// signature that does not verify is [`DbError::SignatureInvalid`]. Uses strict
/// verification, which rejects the malleable non-canonical signature forms.
///
/// # Errors
///
/// [`DbError::UntrustedRoot`] or [`DbError::SignatureInvalid`].
pub fn verify_signature(
    trust: &TrustRoot,
    signature: &PackageSignature,
    message: &[u8],
) -> Result<(), DbError> {
    let public_key = trust
        .public_key(signature.key_id)
        .ok_or(DbError::UntrustedRoot {
            key_id: signature.key_id,
        })?;
    let verifying_key =
        VerifyingKey::from_bytes(&public_key).map_err(|_| DbError::SignatureInvalid)?;
    let sig = Signature::from_bytes(&signature.bytes);
    verifying_key
        .verify_strict(message, &sig)
        .map_err(|_| DbError::SignatureInvalid)
}

#[cfg(test)]
mod tests;
