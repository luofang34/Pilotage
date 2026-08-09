//! Versioned contract for read-only situation queries.
//!
//! A composition host captures one immutable handle for each selected domain.
//! The host reports the identity and the age evidence from each handle.
//! The result uses best-available, non-atomic consistency across domains.

mod compose;
mod conformance;
mod contract;
mod error;
mod evidence;
mod result;
mod snapshot;
mod time;

pub use compose::{ComposingSituationViewV1, SituationViewV1};
pub use conformance::{
    CorpusCaptureV1, CorpusCaseV1, SituationViewCorpusV1, load_corpus_v1, verify_corpus_v1,
    verify_reference_corpus_v1,
};
pub use contract::{
    CoherenceRequirementV1, CoherenceRuleV1, DomainSelectionV1, FieldScopeV1, FreshnessAgeV1,
    FreshnessRequirementV1, QueryAxisV1, QueryRequirementsV1, SituationViewQueryV1,
    SituationViewRequestV1, SnapshotRequirementV1, TimeQueryV1,
};
pub use error::SituationViewError;
pub use evidence::{EvidenceV1, MissingDataReasonV1};
pub use result::{
    AvailableDomainResultV1, ConsistencyGuaranteeV1, ContributorResultV1, DomainResultV1,
    FieldResultV1, RequirementAssessmentV1, RequirementReasonV1, RequirementStatusV1,
    SituationViewResultV1,
};
pub use snapshot::{
    ContributorEvidenceV1, DomainIdentityV1, DomainSnapshotV1, FieldSnapshotV1, SnapshotCaptureV1,
    SnapshotSourceV1,
};
pub use time::{
    AgeUnknownReasonV1, AgeV1, ClockCorrespondenceV1, MonotonicIntervalV1, MonotonicStampV1,
    TimeQualityV1, UtcInstantV1, UtcIntervalV1,
};

/// Schema version for the first request and result contract.
pub const SITUATION_VIEW_SCHEMA_VERSION: u16 = 1;

/// Schema version for the first conformance corpus.
pub const SITUATION_VIEW_CORPUS_VERSION: u16 = 1;
