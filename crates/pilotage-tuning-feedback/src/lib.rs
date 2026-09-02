//! Independent verification for simulator tuning campaign evidence.
//!
//! The verifier reads the simulator-neutral `flight-tune` wire types. It
//! recalculates the complete qualification result without the tuning engine.
//!
//! SIM / NOT FOR FLIGHT.

#![forbid(unsafe_code)]

mod digest;
mod error;
mod evidence;
mod policy;
mod qualification;
mod storage;
mod uncertainty;

pub use error::FeedbackError;
pub use evidence::{
    CAMPAIGN_EVIDENCE_SCHEMA_VERSION, CampaignEvidence, EvidenceReceipt, VerifiedCampaignEvidence,
    VerifiedQualifiedEvidence,
};
pub use policy::RequiredPolicy;
pub use uncertainty::{VerifiedExecutedUncertainty, verify_executed_uncertainty};
