//! Verified X-Plane identity and truth sessions.
//!
//! The X-Plane plugin reports the active aircraft and plugin paths.
//! This crate hashes the files on the host. It does not permit a trial until
//! the actual files match the expected identities.
//!
//! SIM / NOT FOR FLIGHT.

#![forbid(unsafe_code)]

mod build_manifest;
mod client;
mod error;
mod identity;
mod protocol;
mod sample;

pub use build_manifest::TrialPluginBuildManifest;
pub use client::{SessionReceipt, XPlaneTrialListener, XPlaneTrialSession};
pub use error::XPlaneTrialError;
pub use identity::{
    ExpectedArtifact, ExpectedXPlaneIdentity, VerifiedXPlaneBinding, VerifiedXPlaneIdentity,
};
pub use pilotage_trial::Digest;
pub use sample::XPlaneTruthSample;

#[cfg(test)]
mod tests;
