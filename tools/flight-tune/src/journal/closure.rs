use serde::{Deserialize, Serialize};

use crate::identity::digest_bytes;
use crate::{
    Digest, PromotionComparison, PromotionDecision, PromotionSelection, SearchStage, TuneError,
};

/// The supported promotion closure schema.
///
/// The closure embeds the paired comparison, which now states one result group
/// for each promotion scenario and the authority each one kept. A closure at
/// the earlier version held one flat objective map, so the two shapes cannot
/// share a version number.
pub const PROMOTION_CLOSURE_SCHEMA_VERSION: u16 = 2;

const COMPARISON_DOMAIN: &[u8] = b"pilotage.flight-tune.promotion-comparison.v1\0";
const DECISION_DOMAIN: &[u8] = b"pilotage.flight-tune.promotion-decision.v1\0";
const SELECTION_DOMAIN: &[u8] = b"pilotage.flight-tune.promotion-selection.v1\0";
const CLOSURE_DOMAIN: &[u8] = b"pilotage.flight-tune.promotion-closure.v1\0";

/// The replay-computed closure of one hidden promotion round.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PromotionClosure {
    /// The closure schema.
    pub schema_version: u16,
    /// The exact promotion policy identity.
    pub policy_digest: Digest,
    /// The initial evaluation identity, when that attempt completed.
    pub baseline_evaluation_digest: Option<Digest>,
    /// The initial evaluation proof identity, when that attempt completed.
    pub baseline_proof_digest: Option<Digest>,
    /// The frozen evaluation identity, when that attempt completed.
    pub frozen_evaluation_digest: Option<Digest>,
    /// The frozen evaluation proof identity, when that attempt completed.
    pub frozen_proof_digest: Option<Digest>,
    /// The paired comparison, when both attempts passed.
    pub comparison: Option<PromotionComparison>,
    /// The paired comparison identity, when a comparison exists.
    pub comparison_digest: Option<Digest>,
    /// The terminal promotion class.
    pub decision: PromotionDecision,
    /// The terminal promotion class identity.
    pub decision_digest: Digest,
    /// The only candidate authorized for final qualification.
    pub selected_candidate: Option<Digest>,
    /// The decision and candidate selection identity.
    pub selection_digest: Digest,
    /// The identity of the complete promotion closure.
    pub closure_digest: Digest,
}

#[derive(Serialize)]
struct SelectionDocument<'a> {
    decision: &'a PromotionDecision,
    decision_digest: Digest,
    selected_candidate: Option<Digest>,
}

#[derive(Serialize)]
struct ClosureDocument<'a> {
    schema_version: u16,
    policy_digest: Digest,
    baseline_evaluation_digest: Option<Digest>,
    baseline_proof_digest: Option<Digest>,
    frozen_evaluation_digest: Option<Digest>,
    frozen_proof_digest: Option<Digest>,
    comparison: &'a Option<PromotionComparison>,
    comparison_digest: Option<Digest>,
    decision: &'a PromotionDecision,
    decision_digest: Digest,
    selected_candidate: Option<Digest>,
    selection_digest: Digest,
}

impl PromotionClosure {
    pub(crate) fn new(
        policy_digest: Digest,
        baseline: Option<(Digest, Digest)>,
        frozen: Option<(Digest, Digest)>,
        comparison: Option<PromotionComparison>,
        selection: PromotionSelection,
    ) -> Result<Self, TuneError> {
        let comparison_digest = comparison
            .as_ref()
            .map(|value| domain_digest(COMPARISON_DOMAIN, value, "promotion comparison"))
            .transpose()?;
        let decision_digest =
            domain_digest(DECISION_DOMAIN, &selection.decision, "promotion decision")?;
        let selection_digest = selection_digest(
            &selection.decision,
            decision_digest,
            selection.selected_candidate,
        )?;
        let mut closure = Self {
            schema_version: PROMOTION_CLOSURE_SCHEMA_VERSION,
            policy_digest,
            baseline_evaluation_digest: baseline.map(|value| value.0),
            baseline_proof_digest: baseline.map(|value| value.1),
            frozen_evaluation_digest: frozen.map(|value| value.0),
            frozen_proof_digest: frozen.map(|value| value.1),
            comparison,
            comparison_digest,
            decision: selection.decision,
            decision_digest,
            selected_candidate: selection.selected_candidate,
            selection_digest,
            closure_digest: Digest::from_bytes([0; 32]),
        };
        closure.closure_digest = closure.recompute_closure_digest()?;
        closure.validate()?;
        Ok(closure)
    }

    /// Validates all embedded digest bindings and the terminal shape.
    ///
    /// # Errors
    ///
    /// Returns [`TuneError`] when one digest or terminal field differs.
    pub fn validate(&self) -> Result<(), TuneError> {
        if self.schema_version != PROMOTION_CLOSURE_SCHEMA_VERSION
            || self.policy_digest.is_zero()
            || !paired_options(self.baseline_evaluation_digest, self.baseline_proof_digest)
            || !paired_options(self.frozen_evaluation_digest, self.frozen_proof_digest)
            || self.baseline_evaluation_digest.is_some_and(Digest::is_zero)
            || self.baseline_proof_digest.is_some_and(Digest::is_zero)
            || self.frozen_evaluation_digest.is_some_and(Digest::is_zero)
            || self.frozen_proof_digest.is_some_and(Digest::is_zero)
            || !terminal_shape_is_valid(self)
        {
            return Err(invalid("a promotion closure shape is not valid"));
        }
        let comparison_digest = self
            .comparison
            .as_ref()
            .map(|value| domain_digest(COMPARISON_DOMAIN, value, "promotion comparison"))
            .transpose()?;
        let decision_digest = domain_digest(DECISION_DOMAIN, &self.decision, "promotion decision")?;
        let selection_digest =
            selection_digest(&self.decision, decision_digest, self.selected_candidate)?;
        if self.comparison_digest != comparison_digest
            || self.decision_digest != decision_digest
            || self.selection_digest != selection_digest
            || self.closure_digest.is_zero()
            || self.closure_digest != self.recompute_closure_digest()?
        {
            return Err(invalid("a promotion closure digest changed"));
        }
        Ok(())
    }

    /// Validates the paired comparison against its complete stage.
    ///
    /// # Errors
    ///
    /// Returns [`TuneError`] when the policy, scoped limits, or comparison
    /// differ.
    pub fn validate_for(&self, stage: &SearchStage) -> Result<(), TuneError> {
        self.validate()?;
        if self.policy_digest != crate::promotion_policy_digest(&stage.promotion)? {
            return Err(invalid("a promotion closure policy changed"));
        }
        if let Some(comparison) = &self.comparison {
            comparison.validate_for(stage)?;
        }
        Ok(())
    }

    /// Recomputes the identity of the complete closure.
    ///
    /// # Errors
    ///
    /// Returns [`TuneError`] when JSON encoding fails.
    pub fn recompute_closure_digest(&self) -> Result<Digest, TuneError> {
        domain_digest(
            CLOSURE_DOMAIN,
            &ClosureDocument {
                schema_version: self.schema_version,
                policy_digest: self.policy_digest,
                baseline_evaluation_digest: self.baseline_evaluation_digest,
                baseline_proof_digest: self.baseline_proof_digest,
                frozen_evaluation_digest: self.frozen_evaluation_digest,
                frozen_proof_digest: self.frozen_proof_digest,
                comparison: &self.comparison,
                comparison_digest: self.comparison_digest,
                decision: &self.decision,
                decision_digest: self.decision_digest,
                selected_candidate: self.selected_candidate,
                selection_digest: self.selection_digest,
            },
            "promotion closure",
        )
    }
}

fn terminal_shape_is_valid(closure: &PromotionClosure) -> bool {
    match &closure.decision {
        PromotionDecision::Promoted { .. } | PromotionDecision::RejectedNoImprovement { .. } => {
            closure.comparison.is_some()
                && closure.comparison_digest.is_some()
                && closure
                    .selected_candidate
                    .is_some_and(|digest| !digest.is_zero())
        }
        PromotionDecision::RejectedHardGate { gate_id } => {
            !gate_id.trim().is_empty()
                && closure.comparison.is_none()
                && closure.comparison_digest.is_none()
                && closure.selected_candidate.is_none()
        }
        PromotionDecision::Indeterminate { reason } => {
            !reason.trim().is_empty()
                && closure.comparison.is_none()
                && closure.comparison_digest.is_none()
                && closure.selected_candidate.is_none()
        }
    }
}

const fn paired_options(left: Option<Digest>, right: Option<Digest>) -> bool {
    left.is_some() == right.is_some()
}

fn selection_digest(
    decision: &PromotionDecision,
    decision_digest: Digest,
    selected_candidate: Option<Digest>,
) -> Result<Digest, TuneError> {
    domain_digest(
        SELECTION_DOMAIN,
        &SelectionDocument {
            decision,
            decision_digest,
            selected_candidate,
        },
        "promotion selection",
    )
}

fn domain_digest<T: Serialize>(
    domain: &[u8],
    document: &T,
    name: &'static str,
) -> Result<Digest, TuneError> {
    let encoded = serde_json::to_vec(document).map_err(|source| TuneError::Encode {
        document: name,
        source,
    })?;
    let mut bytes = Vec::with_capacity(domain.len().saturating_add(encoded.len()));
    bytes.extend_from_slice(domain);
    bytes.extend_from_slice(&encoded);
    Ok(digest_bytes(&bytes))
}

fn invalid(detail: impl Into<String>) -> TuneError {
    TuneError::InvalidJournal {
        detail: detail.into(),
    }
}
