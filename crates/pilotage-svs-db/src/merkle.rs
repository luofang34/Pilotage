//! The tile-root hash: a domain-separated Merkle tree over the package's tiles.
//!
//! Each tile hashes to a leaf; leaves combine pairwise into a single
//! [`TileRoot`] that the manifest declares and the signature covers. Because the
//! root depends on every leaf, a one-byte change to any tile changes its leaf,
//! propagates up the tree, and makes the recomputed root disagree with the
//! declared one.
//!
//! Leaves and interior nodes carry distinct one-byte domain tags, so a leaf
//! hash can never be reinterpreted as an interior node (the classic Merkle
//! second-preimage confusion). A lone node at an odd level is promoted
//! unchanged rather than hashed against a duplicate of itself.

use sha2::{Digest, Sha256};

/// A 32-byte Merkle-style root over a package's tiles.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TileRoot(pub [u8; 32]);

/// Domain tag for a leaf hash.
const LEAF_DOMAIN: u8 = 0x00;
/// Domain tag for an interior-node hash.
const NODE_DOMAIN: u8 = 0x01;
/// Domain tag for the root of an empty tree (a package with no tiles).
const EMPTY_DOMAIN: u8 = 0x02;

/// The leaf hash of a tile's canonical bytes.
#[must_use]
pub fn tile_leaf_hash(canonical_tile_bytes: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update([LEAF_DOMAIN]);
    hasher.update(canonical_tile_bytes);
    hasher.finalize().into()
}

/// The interior-node hash of two child hashes.
#[must_use]
fn node_hash(left: &[u8; 32], right: &[u8; 32]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update([NODE_DOMAIN]);
    hasher.update(left);
    hasher.update(right);
    hasher.finalize().into()
}

/// The Merkle root over `leaves`, in the given order. Callers pass leaves in a
/// deterministic order (tiles sorted by key), so the root is a pure function of
/// the tile set.
#[must_use]
pub fn merkle_root(leaves: &[[u8; 32]]) -> TileRoot {
    if leaves.is_empty() {
        let mut hasher = Sha256::new();
        hasher.update([EMPTY_DOMAIN]);
        return TileRoot(hasher.finalize().into());
    }
    let mut level: Vec<[u8; 32]> = leaves.to_vec();
    while level.len() > 1 {
        let mut next: Vec<[u8; 32]> = Vec::with_capacity(level.len().div_ceil(2));
        let mut i = 0;
        while i < level.len() {
            if i + 1 < level.len() {
                next.push(node_hash(&level[i], &level[i + 1]));
            } else {
                next.push(level[i]);
            }
            i += 2;
        }
        level = next;
    }
    TileRoot(level[0])
}

#[cfg(test)]
mod tests;
