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
    /// Whether a review must be marked independent to be accepted.
    pub review_requires_independence: bool,
    /// Attributes every recorded result must carry so it is real evidence and
    /// not a placeholder — typically the executed command, the configuration
    /// commit/tree digest, the pinned tool version, and an immutable artifact
    /// reference.
    pub result_required_attrs: Vec<String>,
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
            review_requires_independence: true,
            result_required_attrs: vec![
                "command".to_string(),
                "config-digest".to_string(),
                "tool-version".to_string(),
                "artifact".to_string(),
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
