//! Immutable snapshot capture boundary.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    ClockCorrespondenceV1, DomainSelectionV1, EvidenceV1, MissingDataReasonV1, MonotonicStampV1,
    TimeQualityV1, TimeQueryV1, UtcInstantV1, UtcIntervalV1,
};

/// Source and time evidence for one field contributor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContributorEvidenceV1 {
    /// Source identity or an explicit missing reason.
    pub source_identity: EvidenceV1<String>,
    /// Source epoch or an explicit missing reason.
    pub source_epoch: EvidenceV1<String>,
    /// Local monotonic ingress stamp.
    pub ingress_time: EvidenceV1<MonotonicStampV1>,
    /// Source observation or product time.
    pub source_time: EvidenceV1<UtcInstantV1>,
    /// Quality of the source time.
    pub time_quality: EvidenceV1<TimeQualityV1>,
    /// Source or product validity interval.
    pub validity: EvidenceV1<UtcIntervalV1>,
    /// Domain-owned quality evidence.
    pub data_quality: EvidenceV1<Value>,
    /// Error bound for the source time.
    pub time_uncertainty_nanoseconds: EvidenceV1<u64>,
}

/// One field in an immutable domain snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldSnapshotV1 {
    /// Domain-owned field name.
    pub field: String,
    /// Field value or an explicit missing reason.
    pub value: EvidenceV1<Value>,
    /// Bounded contributors defined by the domain schema.
    pub contributors: Vec<ContributorEvidenceV1>,
}

/// Domain-specific identity that supplements the common snapshot envelope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum DomainIdentityV1 {
    /// The common producer and revision identity is sufficient.
    Common,
    /// Navdata also supplies its cycle, snapshot ID, and digest.
    Navdata {
        /// Published navigation-data cycle.
        cycle: String,
        /// Identity for one immutable built snapshot.
        snapshot_id: String,
        /// Digest of the canonical snapshot content and cycle.
        snapshot_digest: String,
    },
}

/// One immutable domain snapshot with its common envelope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DomainSnapshotV1 {
    /// Stable domain name.
    pub domain: String,
    /// Domain-owned snapshot subject identity.
    pub subject: String,
    /// Version of the complete domain record.
    pub domain_schema_version: u32,
    /// Opaque identity for one continuous domain producer instance.
    pub producer_instance_id: String,
    /// Revision for this subject and producer instance.
    pub snapshot_revision: u64,
    /// Domain-specific identity fields.
    pub domain_identity: DomainIdentityV1,
    /// Source-derived fields in the composed view.
    pub fields: Vec<FieldSnapshotV1>,
    /// Mappings from contributor clocks to the host clock.
    pub clock_correspondences: Vec<ClockCorrespondenceV1>,
}

/// Result of one immutable snapshot capture.
#[derive(Debug, Clone)]
pub enum SnapshotCaptureV1 {
    /// The source returned one retainable immutable handle.
    Available {
        /// Retained snapshot handle.
        snapshot: Arc<DomainSnapshotV1>,
    },
    /// The source cannot return the selected snapshot.
    Missing {
        /// Reason that the selected snapshot is not available.
        reason: MissingDataReasonV1,
    },
}

/// Source that captures one immutable handle for a selected domain.
pub trait SnapshotSourceV1 {
    /// Captures one handle for `selection` at the selected query time.
    fn capture(&self, selection: &DomainSelectionV1, time: &TimeQueryV1) -> SnapshotCaptureV1;
}
