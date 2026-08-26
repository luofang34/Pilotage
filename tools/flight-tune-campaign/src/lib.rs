//! Publication of simulator-neutral tuning campaign evidence.
//!
//! SIM / NOT FOR FLIGHT.

#![forbid(unsafe_code)]

mod alia_policy;
mod error;
mod publish;

pub use alia_policy::{alia250_promotion_policy, alia250_qualification_policy};
pub use error::CampaignError;
pub use publish::publish_journal_evidence_blocking;
