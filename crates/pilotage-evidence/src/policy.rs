//! The declared validation policy the gate applies.
//!
//! The policy is explicit, documented data rather than a hidden rule buried in
//! the checker. It states which relations count as "upstream toward hazards and
//! intended functions" and "downstream toward implementation and results",
//! which node kinds a rooted requirement must reach in each direction, what a
//! recorded result must resolve to, and what a derived requirement must carry.
//! Different node kinds therefore get different permitted roots and leaves — the
//! policy never demands a single `SYS -> HLR -> LLR -> TEST` spine.

use crate::node::NodeKind;
use crate::relation::RelationKind;

/// A declared no-orphan policy.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Policy {
    /// Relations walked in their declared direction when tracing upstream.
    pub upstream_forward: Vec<RelationKind>,
    /// Relations walked against their direction when tracing upstream.
    pub upstream_reverse: Vec<RelationKind>,
    /// Node kinds a rooted requirement must all reach upstream.
    pub upstream_required: Vec<NodeKind>,
    /// Upstream kinds that, if any is reached, root a requirement on their own
    /// (a recorded rationale path in place of a hazard/function path).
    pub upstream_rationale: Vec<NodeKind>,
    /// Relations walked in their declared direction when tracing downstream.
    pub downstream_forward: Vec<RelationKind>,
    /// Relations walked against their direction when tracing downstream.
    pub downstream_reverse: Vec<RelationKind>,
    /// Node kinds a requirement must all reach downstream.
    pub downstream_required: Vec<NodeKind>,
    /// Attributes every derived requirement must carry.
    pub derived_required_attrs: Vec<String>,
    /// Whether a derived requirement must have an incoming review.
    pub derived_requires_review: bool,
    /// The `status` values that count as a completed review. A review whose
    /// status is absent or outside this set (e.g. `pending`, `in-progress`) is
    /// incomplete and cannot yield a VALID verdict.
    pub review_complete_statuses: Vec<String>,
    /// Attributes a completed review must carry so its status is backed by a
    /// substantive record entry, not merely a status string — the reviewer
    /// identity, the date, and the disposition.
    pub review_record_attrs: Vec<String>,
    /// Disposition values that count as a closed review outcome. A completed
    /// review whose disposition is absent or outside this set (e.g. `pending`)
    /// has no real outcome recorded and stays incomplete.
    pub review_closed_dispositions: Vec<String>,
    /// Whether a review must be marked independent to be accepted.
    pub review_requires_independence: bool,
    /// Attributes every recorded result must carry so it is real evidence and
    /// not a placeholder — the executed command, the configuration commit/tree
    /// digest, the pinned tool version, a repo-relative execution-output
    /// artifact path, the digest of that artifact, and the run identity.
    pub result_required_attrs: Vec<String>,
    /// The result attribute that must reference an immutable execution output
    /// (the captured run's recorded result/log), not the test source. A value
    /// using the source-blob scheme is rejected as a placeholder.
    pub result_output_attr: String,
    /// The result attribute naming the committed execution-output artifact file
    /// (repo-relative). Under resolution the gate reads that file, hashes it, and
    /// requires the hash to equal [`result_output_attr`](Self::result_output_attr).
    pub result_artifact_attr: String,
    /// Attributes whose values must also appear, and match, as parsed fields in
    /// the resolved artifact's structured run record. Hash equality alone is not
    /// evidence the run happened with the declared command/config/tool and
    /// produced the declared outcome.
    pub result_artifact_fields: Vec<String>,
    /// Whether an exception must record an independent review to apply.
    pub exception_requires_review: bool,
    /// The ISO-8601 date (`YYYY-MM-DD`) an exception's expiry is checked
    /// against. `None` disables the expiry check so the default policy stays
    /// clock-free and deterministic; a caller that wants expiry enforced injects
    /// an explicit as-of date.
    pub exception_as_of: Option<String>,
    /// The attribute on a verification case naming the executed test symbol.
    pub selector_attr: String,
}

impl Policy {
    /// The declared engineering-trace policy used across the project.
    ///
    /// It is standard-neutral: it enforces that the declared trace is complete
    /// and resolvable, not that any certification objective is satisfied.
    #[must_use]
    pub fn engineering_trace() -> Self {
        use NodeKind::{
            Design, ExternalEvidence, Hazard, Implementation, IntendedFunction, VerificationCase,
            VerificationResult,
        };
        use RelationKind::{
            AllocatedTo, Covers, DecomposesTo, DerivesFrom, ImplementedBy, JustifiedBy, Mitigates,
            ResultOf, Satisfies, VerifiedBy,
        };
        Self {
            upstream_forward: vec![Mitigates, DerivesFrom, JustifiedBy],
            upstream_reverse: vec![DecomposesTo],
            upstream_required: vec![Hazard, IntendedFunction],
            upstream_rationale: vec![ExternalEvidence],
            downstream_forward: vec![AllocatedTo, ImplementedBy, VerifiedBy],
            downstream_reverse: vec![Satisfies, Covers, ResultOf],
            downstream_required: vec![Design, Implementation, VerificationCase, VerificationResult],
            derived_required_attrs: vec![
                "safety-impact".to_string(),
                "rationale".to_string(),
                "disposition".to_string(),
            ],
            derived_requires_review: true,
            review_complete_statuses: vec![
                "complete".to_string(),
                "approved".to_string(),
                "accepted".to_string(),
                "closed".to_string(),
            ],
            review_record_attrs: vec![
                "reviewer".to_string(),
                "date".to_string(),
                "disposition".to_string(),
            ],
            review_closed_dispositions: vec![
                "approved".to_string(),
                "approved with actions".to_string(),
                "approved-with-actions".to_string(),
                "accepted".to_string(),
                "rejected".to_string(),
            ],
            review_requires_independence: true,
            result_required_attrs: vec![
                "command".to_string(),
                "config-digest".to_string(),
                "tool-version".to_string(),
                "artifact".to_string(),
                "output-digest".to_string(),
                "run-id".to_string(),
                "outcome".to_string(),
            ],
            result_output_attr: "output-digest".to_string(),
            result_artifact_attr: "artifact".to_string(),
            result_artifact_fields: vec![
                "command".to_string(),
                "config-digest".to_string(),
                "tool-version".to_string(),
                "run-id".to_string(),
                "outcome".to_string(),
            ],
            exception_requires_review: true,
            exception_as_of: None,
            selector_attr: "test".to_string(),
        }
    }
}

impl Default for Policy {
    fn default() -> Self {
        Self::engineering_trace()
    }
}
