//! Panel registry: the descriptor contract shells compose (ADR-0029,
//! ADR-0033).
//!
//! A panel is a plugin over three stable contracts — the state-group
//! vocabulary, the scene-command IR, and the glyph vocabulary. This
//! crate holds the descriptor a shell consumes instead of hard-coded
//! panel enumeration: identity, required layers and state groups, the
//! design frame, background capability, the bounded key-TLV
//! configuration schema, and the draw entry point. A registry is plain
//! data composed by each shell ([`Registry::new`] validates the
//! composition at init); an out-of-repo panel registers by being listed
//! in the shell's descriptor slice, never by link-time magic.

#![no_std]

#[cfg(test)]
extern crate std;

mod config;
mod descriptor;
mod group_set;
mod registry;

pub use config::{CONFIG_BLOB_MAX, ConfigBlob, ConfigError, ConfigKey, EMPTY_CONFIG, keys};
pub use descriptor::{
    BackgroundCapability, DesignFrame, DrawFn, ExtremeState, PanelDescriptor, PanelDrawError,
    Region,
};
pub use group_set::GroupSet;
pub use registry::{Registry, RegistryError};
