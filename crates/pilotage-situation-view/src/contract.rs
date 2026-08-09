//! Caller query and host-attached request types.

use serde::{Deserialize, Serialize};

use crate::{MonotonicStampV1, SITUATION_VIEW_SCHEMA_VERSION, UtcInstantV1};

/// Time axis fixed by a query.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QueryAxisV1 {
    /// Select data that is in effect at the UTC value.
    ValidTime,
    /// Select data that the system held at the UTC value.
    KnowledgeTime,
}

/// The time selection for one query.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimeQueryV1 {
    /// Axis fixed by the query.
    pub axis: QueryAxisV1,
    /// UTC value on the selected axis.
    pub evaluation_utc: UtcInstantV1,
}

/// One domain and subject selected by the caller.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct DomainSelectionV1 {
    /// Stable domain name.
    pub domain: String,
    /// Domain-owned snapshot subject identity.
    pub subject: String,
}

/// One field in a selected domain snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldScopeV1 {
    /// Stable domain name.
    pub domain: String,
    /// Domain-owned snapshot subject identity.
    pub subject: String,
    /// Domain-owned field name.
    pub field: String,
}

/// Age used by a freshness requirement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FreshnessAgeV1 {
    /// Use the age from local ingress.
    Ingress,
    /// Use the age from the source observation time.
    Observation,
}

/// Maximum age for one field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FreshnessRequirementV1 {
    /// Caller-owned requirement identity.
    pub requirement_id: String,
    /// Field to assess.
    pub field: FieldScopeV1,
    /// Age to compare.
    pub age: FreshnessAgeV1,
    /// Largest accepted age, including its uncertainty bound.
    pub maximum_age_nanoseconds: u64,
}

/// Required identity for one captured snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotRequirementV1 {
    /// Stable domain name.
    pub domain: String,
    /// Domain-owned snapshot subject identity.
    pub subject: String,
    /// Required producer instance ID.
    pub producer_instance_id: String,
    /// Required snapshot revision.
    pub snapshot_revision: u64,
}

/// Rule for one coherence requirement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum CoherenceRuleV1 {
    /// Limit the possible ingress-age spread for a set of fields.
    MaximumIngressAgeSpread {
        /// Fields that must be coherent.
        fields: Vec<FieldScopeV1>,
        /// Largest accepted spread.
        maximum_spread_nanoseconds: u64,
    },
    /// Require a set of fields to contain equal JSON values.
    EqualFieldValues {
        /// Fields that must contain the same value.
        fields: Vec<FieldScopeV1>,
    },
    /// Require exact producer instance IDs and revisions.
    ExactSnapshots {
        /// Snapshot identities that the result must contain.
        snapshots: Vec<SnapshotRequirementV1>,
    },
}

/// One caller-supplied coherence requirement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoherenceRequirementV1 {
    /// Caller-owned requirement identity.
    pub requirement_id: String,
    /// Coherence rule to assess.
    #[serde(flatten)]
    pub rule: CoherenceRuleV1,
}

/// Freshness and coherence requirements for one query.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct QueryRequirementsV1 {
    /// Field freshness requirements.
    pub freshness: Vec<FreshnessRequirementV1>,
    /// Cross-field or cross-domain coherence requirements.
    pub coherence: Vec<CoherenceRequirementV1>,
}

/// Caller-owned part of a situation query.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SituationViewQueryV1 {
    /// Time axis and UTC value.
    pub time: TimeQueryV1,
    /// Domains that the caller selects.
    pub domains: Vec<DomainSelectionV1>,
    /// Caller freshness and coherence requirements.
    pub requirements: QueryRequirementsV1,
}

/// Versioned request after the composition host attaches its clock stamp.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SituationViewRequestV1 {
    /// Contract schema version.
    pub schema_version: u16,
    /// Caller-owned query.
    pub query: SituationViewQueryV1,
    /// Host monotonic stamp at evaluation.
    pub host_evaluation: MonotonicStampV1,
}

impl SituationViewRequestV1 {
    /// Attaches the host evaluation stamp to a caller query.
    #[must_use]
    pub const fn attach(query: SituationViewQueryV1, host_evaluation: MonotonicStampV1) -> Self {
        Self {
            schema_version: SITUATION_VIEW_SCHEMA_VERSION,
            query,
            host_evaluation,
        }
    }
}
