//! Tests for the domain-separated Merkle tile-root.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use super::{merkle_root, tile_leaf_hash};

fn leaf(byte: u8) -> [u8; 32] {
    tile_leaf_hash(&[byte; 8])
}

#[test]
fn changing_one_leaf_changes_the_root() {
    let base = [leaf(1), leaf(2), leaf(3)];
    let mutated = [leaf(1), leaf(2), leaf(4)];
    assert_ne!(merkle_root(&base), merkle_root(&mutated));
}

#[test]
fn distinct_leaf_sets_have_distinct_roots() {
    let a = [leaf(1), leaf(2)];
    let b = [leaf(3), leaf(4)];
    assert_ne!(merkle_root(&a), merkle_root(&b));
}

#[test]
fn empty_tree_root_is_fixed_and_distinct_from_any_leaf() {
    let empty = merkle_root(&[]);
    assert_eq!(empty, merkle_root(&[]));
    assert_ne!(empty.0, leaf(0));
    assert_ne!(empty.0, leaf(1));
}

#[test]
fn leaf_and_interior_node_domains_do_not_collide() {
    // A single-leaf tree yields the leaf; a two-leaf tree yields an interior
    // node hash. Distinct domains keep them apart even for related inputs.
    let single = merkle_root(&[leaf(5)]);
    let paired = merkle_root(&[leaf(5), leaf(5)]);
    assert_ne!(single, paired);
}

#[test]
fn root_is_a_pure_function_of_leaf_order() {
    let leaves = [leaf(1), leaf(2), leaf(3)];
    assert_eq!(merkle_root(&leaves), merkle_root(&leaves));
}
