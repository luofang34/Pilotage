//! Generated `prost` wire types for the `pilotage.v1` package (ADR-0014).
//!
//! The generated code is produced by `build.rs` from `schemas/pilotage/v1`
//! and is neither hand-edited nor committed; `missing_docs` is allowed here
//! because `prost` does not emit doc comments for every generated item, and
//! the schema `.proto` files (the source of truth) already carry the
//! documentation.
//!
//! `large_enum_variant` is allowed on the generated code: the `Envelope`
//! payload is a message-routing union that only ever holds one variant at
//! a time and is heap-owned in the transport, so boxing every large
//! variant to equalize inline sizes is churn that fights prost's shape.
//! The large per-group fields inside `TelemetrySample` are already boxed
//! (`build.rs`).
#![allow(missing_docs, clippy::large_enum_variant)]

include!(concat!(env!("OUT_DIR"), "/pilotage.v1.rs"));
