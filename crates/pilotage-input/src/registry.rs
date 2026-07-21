//! Versioned device-profile registry: layered precedence and sans-IO
//! loading (ADR-0007).
//!
//! The `registry/` directory alongside this module holds built-in profile
//! JSON files, one per device, checked into the crate and embedded via
//! `include_str!` (see `loader::GENERIC_GAMEPAD_JSON`). Additional built-in
//! devices are added as new files there, not by editing this module.

mod layer;
mod loader;
mod merge;
mod select;

pub use layer::{LayeredProfile, ProfileLayer};
pub use loader::{
    GENERIC_GAMEPAD_JSON, load_builtin_generic_gamepad, load_profile_bytes, load_profile_str,
};
pub use merge::{layered, merge_layers};
pub use select::select_by_identity;
