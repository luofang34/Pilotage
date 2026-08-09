//! Versioned result and requirement assessment types.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    AgeV1, ClockCorrespondenceV1, ContributorEvidenceV1, DomainIdentityV1, MissingDataReasonV1,
    MonotonicStampV1, TimeQueryV1,
};

/// Cross-domain consistency supplied by the first contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConsistencyGuaranteeV1 {
    /// Each domain has one immutable handle. Domains are not atomic together.
    BestAvailableNonAtomic,
}

/// Contributor evidence with its two separate ages.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContributorResultV1 {
    /// Source, epoch, time, validity, quality, and uncertainty evidence.
    #[serde(flatten)]
    pub evidence: ContributorEvidenceV1,
    /// Age from local ingress to host evaluation.
    pub ingress_age: AgeV1,
    /// Age from source observation to evaluation UTC.
    pub observation_age: AgeV1,
}

/// One composed field and its contributors.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldResultV1 {
    /// Domain-owned field name.
    pub field: String,
    /// Field value or an explicit missing reason.
    pub value: crate::EvidenceV1<Value>,
    /// Bounded source contributors.
    pub contributors: Vec<ContributorResultV1>,
}

/// Available domain result with its snapshot identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AvailableDomainResultV1 {
    /// Version of the complete domain record.
    pub domain_schema_version: u32,
    /// Opaque identity for one continuous domain producer instance.
    pub producer_instance_id: String,
    /// Revision for this subject and producer instance.
    pub snapshot_revision: u64,
    /// Domain-specific identity fields.
    pub domain_identity: DomainIdentityV1,
    /// Composed fields.
    pub fields: Vec<FieldResultV1>,
    /// Clock mappings supplied with the captured snapshot.
    pub clock_correspondences: Vec<ClockCorrespondenceV1>,
}

/// Result for one selected domain.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "state")]
pub enum DomainResultV1 {
    /// The view captured one immutable snapshot handle.
    Available {
        /// Stable domain name.
        domain: String,
        /// Domain-owned snapshot subject identity.
        subject: String,
        /// Available snapshot result.
        result: AvailableDomainResultV1,
    },
    /// The selected domain snapshot is not available.
    Missing {
        /// Stable domain name.
        domain: String,
        /// Domain-owned snapshot subject identity.
        subject: String,
        /// Reason that the selected domain is missing.
        reason: MissingDataReasonV1,
    },
}

/// Reason for a failed or indeterminate requirement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RequirementReasonV1 {
    /// A selected domain is missing.
    MissingDomain,
    /// A selected field is missing.
    MissingField,
    /// A field has no contributor evidence.
    MissingContributor,
    /// A required age is unknown.
    UnknownAge,
    /// An age uncertainty is not available.
    UnknownUncertainty,
    /// A maximum age is exceeded.
    MaximumAgeExceeded,
    /// A maximum age spread is exceeded.
    MaximumSpreadExceeded,
    /// Required field values are different.
    FieldValuesDiffer,
    /// A producer instance ID or revision is different.
    SnapshotIdentityMismatch,
}

/// Assessment state for one caller requirement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "state")]
pub enum RequirementStatusV1 {
    /// The result satisfies the requirement.
    Satisfied,
    /// The result does not satisfy the requirement.
    NotSatisfied {
        /// Reason that the requirement failed.
        reason: RequirementReasonV1,
    },
    /// The available evidence cannot decide the requirement.
    Indeterminate {
        /// Reason that the assessment is indeterminate.
        reason: RequirementReasonV1,
    },
}

/// Assessment for one caller requirement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequirementAssessmentV1 {
    /// Caller-owned requirement identity.
    pub requirement_id: String,
    /// Requirement assessment.
    #[serde(flatten)]
    pub status: RequirementStatusV1,
}

/// Complete versioned result for one request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SituationViewResultV1 {
    /// Contract schema version.
    pub schema_version: u16,
    /// Query axis and UTC value from the request.
    pub query_time: TimeQueryV1,
    /// Host monotonic evaluation stamp from the request.
    pub host_evaluation: MonotonicStampV1,
    /// Cross-domain consistency guarantee.
    pub consistency: ConsistencyGuaranteeV1,
    /// Results in the same order as the selected domains.
    pub domains: Vec<DomainResultV1>,
    /// Assessments in request order.
    pub requirement_assessments: Vec<RequirementAssessmentV1>,
}
