use std::collections::HashSet;

use flight_tune::{
    AttemptRole, AuthenticatedEvaluationProof, CandidateEvaluation, Digest, HardGateFailure,
    RunRecord, RunTerminalCompletion, RunTerminalDisposition, RunTerminalQuarantine,
    RunTerminalReceipt, RunTerminalSemanticOutcome, SearchStage,
};
use serde::Serialize;

use crate::{FeedbackError, digest, error::invalid};

use super::{authority::AttemptAuthority, plan, statistics};

const EVALUATION_DOMAIN: &[u8] = b"pilotage.flight-tune.authenticated-evaluation.v1\0";
const PROOF_DOMAIN: &[u8] = b"pilotage.flight-tune.authenticated-evaluation-proof.v1\0";
const PROOF_SCHEMA_VERSION: u16 = 2;

pub(super) struct VerifiedProof<'a> {
    pub(super) proof: &'a AuthenticatedEvaluationProof,
}

#[derive(Serialize)]
struct EvaluationDocument<'a> {
    schema_version: u16,
    trial_id: u64,
    role: AttemptRole,
    candidate_digest: Digest,
    plan_digest: Digest,
    retry_index: u32,
    evaluation: &'a CandidateEvaluation,
}

#[derive(Serialize)]
struct ProofDocument<'a> {
    schema_version: u16,
    evaluation_digest: Digest,
    receipt_digests: &'a [Digest],
}

pub(super) fn verify<'a>(
    proof: &'a AuthenticatedEvaluationProof,
    stage: &SearchStage,
    session: &flight_tune::SessionIdentity,
    session_digest: Digest,
    attempt: AttemptAuthority,
) -> Result<VerifiedProof<'a>, FeedbackError> {
    verify_header(proof, attempt)?;
    let expected = plan::expected_runs(
        stage,
        attempt.role,
        attempt.candidate,
        attempt.trial_id,
        session.fixed_seed,
        session_digest,
        attempt.retry_index,
    )?;
    verify_receipt_headers(proof, session, session_digest, attempt)?;
    match &proof.evaluation {
        CandidateEvaluation::Passed { aggregate, runs } => {
            verify_passed(stage, attempt.role, &expected, proof, runs)?;
            if aggregate != &statistics::aggregate(runs)? {
                return Err(invalid("a saved evaluation aggregate changed"));
            }
        }
        CandidateEvaluation::HardGateFailed {
            failure,
            completed_runs,
        } => verify_hard_gate(
            stage,
            attempt.role,
            &expected,
            proof,
            completed_runs,
            failure,
        )?,
        CandidateEvaluation::Quarantined { reason } => {
            verify_quarantine(stage, attempt.role, &expected, proof, reason)?;
        }
    }
    verify_digests(proof)?;
    Ok(VerifiedProof { proof })
}

fn verify_header(
    proof: &AuthenticatedEvaluationProof,
    attempt: AttemptAuthority,
) -> Result<(), FeedbackError> {
    if proof.schema_version != PROOF_SCHEMA_VERSION
        || proof.trial_id != attempt.trial_id
        || proof.role != attempt.role
        || proof.candidate_digest != attempt.candidate
        || attempt.candidate.is_zero()
        || proof.plan_digest != attempt.plan_digest
        || proof.plan_digest.is_zero()
        || proof.retry_index != attempt.retry_index
        || proof.terminal_receipts.is_empty()
        || !is_hidden(attempt.role)
    {
        return Err(invalid("an authenticated evaluation proof header changed"));
    }
    Ok(())
}

fn verify_receipt_headers(
    proof: &AuthenticatedEvaluationProof,
    session: &flight_tune::SessionIdentity,
    session_digest: Digest,
    attempt: AttemptAuthority,
) -> Result<(), FeedbackError> {
    let mut identities = HashSet::new();
    for receipt in &proof.terminal_receipts {
        ::flight_tune::RunTerminalReceipt::validate(receipt)
            .map_err(|error| invalid(format!("a terminal receipt is not valid: {error}")))?;
        let context = receipt.context();
        if context.tuning_session_digest() != session_digest
            || context.trial_id() != attempt.trial_id
            || context.role() != attempt.role
            || context.candidate_digest() != attempt.candidate
            || context.retry_index() != attempt.retry_index
            || receipt.binding().adapter() != &session.runtimes.vehicle
            || !identities.insert(receipt.receipt_digest())
        {
            return Err(invalid(
                "an evaluation receipt identity changed or repeated",
            ));
        }
    }
    Ok(())
}

fn verify_passed(
    stage: &SearchStage,
    role: AttemptRole,
    expected: &[plan::ExpectedRun],
    proof: &AuthenticatedEvaluationProof,
    runs: &[RunRecord],
) -> Result<(), FeedbackError> {
    if runs.len() != expected.len() || proof.terminal_receipts.len() != expected.len() {
        return Err(invalid("a passing evaluation has an incomplete run plan"));
    }
    for ((expected_run, receipt), run) in expected
        .iter()
        .zip(&proof.terminal_receipts)
        .zip(runs)
    {
        verify_completed(stage, role, receipt, run, expected_run)?;
    }
    Ok(())
}

fn verify_hard_gate(
    stage: &SearchStage,
    role: AttemptRole,
    expected: &[plan::ExpectedRun],
    proof: &AuthenticatedEvaluationProof,
    completed: &[RunRecord],
    failure: &HardGateFailure,
) -> Result<(), FeedbackError> {
    if completed.len() >= expected.len()
        || proof.terminal_receipts.len() != completed.len().wrapping_add(1)
    {
        return Err(invalid("a hard-gate evaluation has an invalid run prefix"));
    }
    for ((expected_run, receipt), run) in expected
        .iter()
        .zip(&proof.terminal_receipts)
        .zip(completed)
    {
        verify_completed(stage, role, receipt, run, expected_run)?;
    }
    let index = completed.len();
    let receipt = &proof.terminal_receipts[index];
    let expected_run = &expected[index];
    plan::verify_receipt_context(receipt, expected_run)?;
    verify_failure(stage, receipt, failure, expected_run)
}

fn verify_quarantine(
    stage: &SearchStage,
    role: AttemptRole,
    expected: &[plan::ExpectedRun],
    proof: &AuthenticatedEvaluationProof,
    reason: &str,
) -> Result<(), FeedbackError> {
    let Some((last, prefix)) = proof.terminal_receipts.split_last() else {
        return Err(invalid("a quarantine proof has no terminal receipt"));
    };
    if reason.trim().is_empty() || proof.terminal_receipts.len() > expected.len() {
        return Err(invalid("a quarantine evaluation has an invalid run prefix"));
    }
    for (index, receipt) in prefix.iter().enumerate() {
        let run = completed_run(receipt)
            .ok_or_else(|| invalid("a quarantine prefix contains a non-completed run"))?;
        verify_completed(stage, role, receipt, run, &expected[index])?;
    }
    plan::verify_receipt_context(last, &expected[prefix.len()])?;
    let RunTerminalDisposition::Quarantine { quarantine } = last.class().disposition() else {
        return Err(invalid("a quarantine evaluation has a completed receipt"));
    };
    if reason != quarantine_reason(last.receipt_digest(), quarantine) {
        return Err(invalid("a quarantine evaluation reason changed"));
    }
    Ok(())
}

fn verify_completed(
    stage: &SearchStage,
    role: AttemptRole,
    receipt: &RunTerminalReceipt,
    run: &RunRecord,
    expected: &plan::ExpectedRun,
) -> Result<(), FeedbackError> {
    plan::verify_receipt_context(receipt, expected)?;
    let saved = completed_run(receipt)
        .ok_or_else(|| invalid("an evaluation run is not a completed scenario receipt"))?;
    if saved != run {
        return Err(invalid(
            "an evaluation run changed from its terminal receipt",
        ));
    }
    verify_run(stage, role, run)
}

fn verify_run(
    stage: &SearchStage,
    role: AttemptRole,
    run: &RunRecord,
) -> Result<(), FeedbackError> {
    if run.scenario_set != plan::scenario_set(role)
        || !run.loss.is_finite()
        || run.loss < 0.0
        || !run.control_effort.is_finite()
        || !(0.0..=1.0).contains(&run.control_effort)
        || run.passed_hard_gates != stage.required_hard_gates
        || !objective_keys_match(stage, role, run)
    {
        return Err(invalid(
            "an evaluation run changed its metrics, gates, or objectives",
        ));
    }
    verify_objectives(&run.objectives)
}

/// Requires the objective names and values the core would have accepted.
///
/// A name that carries whitespace can present as another name once a report
/// renders it, so the core refuses one and this verifier refuses the same.
///
/// # Errors
///
/// Returns [`FeedbackError`] when a name is blank, carries whitespace, or a
/// value is not finite and nonnegative.
pub(super) fn verify_objectives(
    objectives: &std::collections::BTreeMap<String, f64>,
) -> Result<(), FeedbackError> {
    if objectives.iter().any(|(name, value)| {
        name.trim().is_empty()
            || name.chars().any(char::is_whitespace)
            || !value.is_finite()
            || *value < 0.0
    }) {
        return Err(invalid(
            "a named objective is empty, carries whitespace, or is out of range",
        ));
    }
    Ok(())
}

fn verify_failure(
    stage: &SearchStage,
    receipt: &RunTerminalReceipt,
    failure: &HardGateFailure,
    expected: &plan::ExpectedRun,
) -> Result<(), FeedbackError> {
    let saved = completed_failure(receipt)
        .ok_or_else(|| invalid("a hard-gate result has no completed abort receipt"))?;
    let ordinary_gate = stage
        .required_hard_gates
        .iter()
        .any(|gate| gate == &failure.gate.id)
        && failure.sample_sequence < u64::from(expected.scenario.max_samples);
    let core_gate = match failure.gate.id.as_str() {
        "core.no_samples" => Some(failure.sample_sequence == 0 && failure.elapsed_ms == 0),
        "core.sample_limit" => {
            Some(failure.sample_sequence == u64::from(expected.scenario.max_samples))
        }
        "core.sample_timeout" => {
            Some(failure.sample_sequence <= u64::from(expected.scenario.max_samples))
        }
        _ => None,
    };
    if saved != failure
        || failure.scenario_set != expected.scenario_set
        || failure.mission_revision_id != expected.scenario.revision_id
        || failure.repetition != expected.repetition
        || failure.seed != expected.seed
        || failure.gate.passed
        || failure.gate.detail.trim().is_empty()
        || !core_gate.unwrap_or(ordinary_gate)
    {
        return Err(invalid("a hard-gate failure changed from its run plan"));
    }
    Ok(())
}

fn verify_digests(proof: &AuthenticatedEvaluationProof) -> Result<(), FeedbackError> {
    let evaluation_digest = digest::domain(
        "authenticated evaluation",
        EVALUATION_DOMAIN,
        &EvaluationDocument {
            schema_version: proof.schema_version,
            trial_id: proof.trial_id,
            role: proof.role,
            candidate_digest: proof.candidate_digest,
            plan_digest: proof.plan_digest,
            retry_index: proof.retry_index,
            evaluation: &proof.evaluation,
        },
    )?;
    let receipt_digests = proof
        .terminal_receipts
        .iter()
        .map(RunTerminalReceipt::receipt_digest)
        .collect::<Vec<_>>();
    let proof_digest = digest::domain(
        "authenticated evaluation proof",
        PROOF_DOMAIN,
        &ProofDocument {
            schema_version: proof.schema_version,
            evaluation_digest: proof.evaluation_digest,
            receipt_digests: &receipt_digests,
        },
    )?;
    if proof.evaluation_digest.is_zero()
        || proof.proof_digest.is_zero()
        || proof.evaluation_digest != evaluation_digest
        || proof.proof_digest != proof_digest
    {
        return Err(invalid("an authenticated evaluation proof digest changed"));
    }
    Ok(())
}

fn objective_keys_match(stage: &SearchStage, role: AttemptRole, run: &RunRecord) -> bool {
    match role {
        AttemptRole::PromotionBaseline | AttemptRole::PromotionFrozen => run
            .objectives
            .keys()
            .eq(stage.promotion.objective_regression_upper_95.keys()),
        AttemptRole::FinalQualification => run
            .objectives
            .keys()
            .eq(stage.qualification.objective_maxima.keys()),
        AttemptRole::TrainingBaseline { .. } | AttemptRole::TrainingChallenger { .. } => false,
    }
}

fn completed_run(receipt: &RunTerminalReceipt) -> Option<&RunRecord> {
    if !matches!(
        receipt.class().disposition(),
        RunTerminalDisposition::Completed {
            completion: RunTerminalCompletion::ScenarioComplete
        }
    ) {
        return None;
    }
    match receipt.intent().outcome() {
        RunTerminalSemanticOutcome::ScenarioComplete { run, .. } => Some(run),
        _ => None,
    }
}

fn completed_failure(receipt: &RunTerminalReceipt) -> Option<&HardGateFailure> {
    if !matches!(
        receipt.class().disposition(),
        RunTerminalDisposition::Completed {
            completion: RunTerminalCompletion::HardGateAbort
        }
    ) {
        return None;
    }
    match receipt.intent().outcome() {
        RunTerminalSemanticOutcome::HardGateAbort { failure, .. } => Some(failure),
        _ => None,
    }
}

fn quarantine_reason(digest: Digest, class: RunTerminalQuarantine) -> String {
    let name = match class {
        RunTerminalQuarantine::TerminalFailure => "terminal_failure",
        RunTerminalQuarantine::ExecutionFailure => "execution_failure",
        RunTerminalQuarantine::Recovery => "recovery",
        RunTerminalQuarantine::EvidenceFailure => "evidence_failure",
    };
    format!("terminal receipt {digest} has quarantine class {name}")
}

const fn is_hidden(role: AttemptRole) -> bool {
    matches!(
        role,
        AttemptRole::PromotionBaseline
            | AttemptRole::PromotionFrozen
            | AttemptRole::FinalQualification
    )
}
