//! Generated `prost` wire types for the internal `pilotage.bridge.v1`
//! package (ADR-0008).
//!
//! Produced by `build.rs` from `schemas/pilotage/bridge/v1/bridge.proto`
//! and neither hand-edited nor committed; `missing_docs` is allowed here
//! because `prost` does not emit doc comments for every generated item, and
//! the schema `.proto` file (the source of truth) already carries the
//! documentation. This is the host<->sidecar bridge wire format only — it
//! MUST NOT be re-exported into the public client protocol surface.
#![allow(missing_docs)]

include!(concat!(env!("OUT_DIR"), "/pilotage.bridge.v1.rs"));
