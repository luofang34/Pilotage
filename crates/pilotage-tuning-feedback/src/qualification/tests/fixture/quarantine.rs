//! One quarantined hidden proof, built from a quarantine terminal receipt.

use flight_tune::{
    AttemptRole, AuthenticatedEvaluationProof, CandidateEvaluation, Digest, RunTerminalDisposition,
    RunTerminalQuarantine, SearchStage, SessionIdentity,
};

use crate::digest;

use super::super::super::plan;
use super::terminal::{quarantine_receipt, run};
use super::{Point, refresh_proof};

pub(in crate::qualification::tests) fn quarantined_proof(
    stage: &SearchStage,
    session: &SessionIdentity,
    trial_id: u64,
    role: AttemptRole,
    candidate: Digest,
) -> AuthenticatedEvaluationProof {
    let session_digest = digest::document("session identity", session).expect("session digest");
    let expected = plan::expected_runs(
        stage,
        role,
        candidate,
        trial_id,
        session.fixed_seed,
        session_digest,
        0,
    )
    .expect("the expected run plan");
    let run = run(
        &expected[0],
        Point {
            loss: 0.8,
            effort: 0.35,
            objective: 0.21,
        },
        false,
    );
    let terminal = quarantine_receipt(&expected[0], run, &session.runtimes.vehicle);
    let RunTerminalDisposition::Quarantine { quarantine } = terminal.class().disposition() else {
        panic!("fixture receipt must quarantine");
    };
    let reason = quarantine_reason(terminal.receipt_digest(), quarantine);
    let mut proof = AuthenticatedEvaluationProof {
        retry_index: 0,
        schema_version: flight_tune::AUTHENTICATED_EVALUATION_PROOF_SCHEMA_VERSION,
        trial_id,
        role,
        candidate_digest: candidate,
        plan_digest: plan::digest_for(stage, role, candidate, session.fixed_seed)
            .expect("run plan digest"),
        evaluation: CandidateEvaluation::Quarantined { reason },
        terminal_receipts: vec![terminal],
        evaluation_digest: Digest::from_bytes([0; 32]),
        proof_digest: Digest::from_bytes([0; 32]),
    };
    refresh_proof(&mut proof);
    proof
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
