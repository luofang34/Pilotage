use flight_tune::{
    AttemptProjection, AttemptRole, AuthenticatedEvaluationProof, AuthenticatedJournalHead,
    AuthenticatedJournalRecord, CAMPAIGN_EVIDENCE_AUTHORITY_SCHEMA_VERSION,
    CampaignEvidenceAuthority, CandidateTransitionReference, Digest, FinalQualificationOutcome,
    JournalEntry, JournalEvent, JournalEvidenceSnapshot, OperationStatus, PromotionClosure,
    RunTerminalReceipt, SearchStage, SessionIdentity,
};

use crate::{CampaignEvidence, digest};

use super::attempts::training_attempts;

/// The one group and suite the golden stage declares.
fn group_binding(stage: &SearchStage) -> flight_tune::SearchGroupBinding {
    flight_tune::SearchGroupBinding {
        group_id: "golden-group".to_owned(),
        suite_id: "golden-suite".to_owned(),
        suite_index: 0,
        suite_digest: stage.training_suites[0]
            .digest()
            .expect("the golden suite digest"),
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn sealed_campaign(
    stage: SearchStage,
    session: SessionIdentity,
    baseline: AuthenticatedEvaluationProof,
    frozen: AuthenticatedEvaluationProof,
    closure: PromotionClosure,
    final_proof: AuthenticatedEvaluationProof,
    frozen_candidate: Digest,
) -> CampaignEvidence {
    let (authority, head) = campaign_authority(
        &stage,
        &session,
        &baseline,
        &frozen,
        &closure,
        &final_proof,
        frozen_candidate,
    );
    CampaignEvidence::new(JournalEvidenceSnapshot {
        schema_version: flight_tune::JOURNAL_EVIDENCE_SNAPSHOT_SCHEMA_VERSION,
        stage,
        head,
        authority,
        promotion_baseline: baseline,
        promotion_frozen: Some(frozen),
        promotion_closure: closure,
        final_proof: Some(final_proof),
        final_outcome: Some(FinalQualificationOutcome::Qualified),
    })
    .expect("create verified campaign fixture")
}

#[allow(clippy::too_many_arguments)]
fn campaign_authority(
    stage: &SearchStage,
    session: &SessionIdentity,
    baseline: &AuthenticatedEvaluationProof,
    frozen: &AuthenticatedEvaluationProof,
    closure: &PromotionClosure,
    final_proof: &AuthenticatedEvaluationProof,
    frozen_candidate: Digest,
) -> (CampaignEvidenceAuthority, AuthenticatedJournalHead) {
    let session_digest = digest::document("session identity", session).expect("session digest");
    let (training, transition, challenger) =
        training_attempts(stage, session, session_digest, frozen_candidate);
    let mut chain = JournalChain::new(session.clone());
    chain.push(JournalEvent::Started {
        candidate: session.initial_candidate_digest,
    });
    chain.append_attempt(&training, false, Some(true), None);
    chain.push(JournalEvent::CandidateTransitionAuthorized {
        attempt_index: 0,
        reason: "fixture challenger".to_owned(),
        candidate: frozen_candidate,
        group: group_binding(stage),
        receipt: transition.0,
    });
    chain.append_attempt(&challenger, false, Some(true), Some(transition.1));
    let frozen_record = chain.push(JournalEvent::Frozen {
        baseline: session.initial_candidate_digest,
        candidate: frozen_candidate,
    });
    let baseline_record = chain.append_attempt(baseline, true, None, None);
    let frozen_attempt = chain.append_attempt(frozen, true, None, None);
    chain.push(JournalEvent::PromotionClosed {
        closure: closure.clone(),
    });
    let final_attempt = chain.append_attempt(final_proof, true, None, None);
    let sealed = chain.push(JournalEvent::Sealed {
        candidate: frozen_candidate,
        outcome: FinalQualificationOutcome::Qualified,
        promotion_closure_digest: closure.closure_digest,
        final_evaluation_digest: final_proof.evaluation_digest,
        final_proof_digest: final_proof.proof_digest,
    });
    let head = AuthenticatedJournalHead {
        entry: sealed.entry.clone(),
        entry_digest: sealed.entry_digest,
    };
    (
        CampaignEvidenceAuthority {
            schema_version: CAMPAIGN_EVIDENCE_AUTHORITY_SCHEMA_VERSION,
            attempts: AttemptProjection::from_journal_chain(
                &chain.records,
                stage.execution_retry.execution_retry_limit,
            )
            .expect("derive the fixture attempt projection"),
            journal_chain: chain.records,
            candidates: vec![super::tuning_candidate(0.0), super::tuning_candidate(0.5)],
            baseline_candidate: session.initial_candidate_digest,
            frozen_candidate,
            final_candidate: Some(frozen_candidate),
            frozen: frozen_record,
            promotion_baseline: baseline_record,
            promotion_frozen: Some(frozen_attempt),
            final_qualification: Some(final_attempt),
        },
        head,
    )
}

struct JournalChain {
    session: SessionIdentity,
    records: Vec<AuthenticatedJournalRecord>,
}

impl JournalChain {
    fn new(session: SessionIdentity) -> Self {
        Self {
            session,
            records: Vec::new(),
        }
    }

    fn append_attempt(
        &mut self,
        proof: &AuthenticatedEvaluationProof,
        authenticated: bool,
        selected: Option<bool>,
        transition: Option<CandidateTransitionReference>,
    ) -> AuthenticatedJournalRecord {
        let prepared = self.push(JournalEvent::AttemptPrepared {
            trial_id: proof.trial_id,
            role: proof.role,
            candidate: proof.candidate_digest,
            plan_digest: proof.plan_digest,
            transition,
        });
        for (index, receipt) in proof.terminal_receipts.iter().enumerate() {
            let run_index = u64::try_from(index).expect("run index");
            self.append_terminal_run(proof.trial_id, run_index, receipt);
        }
        self.push(JournalEvent::AttemptCompleted {
            trial_id: proof.trial_id,
            evaluation: proof.evaluation.clone(),
            proof: authenticated.then(|| Box::new(proof.clone())),
            selected_as_training_incumbent: selected,
        });
        self.push(JournalEvent::CleanupRecorded {
            trial_id: proof.trial_id,
            cleanup: OperationStatus::Succeeded,
        });
        prepared
    }

    fn append_terminal_run(&mut self, trial_id: u64, run_index: u64, receipt: &RunTerminalReceipt) {
        self.push(JournalEvent::RunPrepared {
            trial_id,
            run_index,
            context: receipt.context().clone(),
            run_intent_digest: receipt.intent().run_intent_digest(),
        });
        self.push(JournalEvent::RunBound {
            trial_id,
            run_index,
            terminal_plan: receipt.report().plan().clone(),
            binding: receipt.binding().clone(),
        });
        self.push(JournalEvent::RunTerminalIntentPrepared {
            trial_id,
            run_index,
            intent: receipt.intent().clone(),
        });
        self.push(JournalEvent::RunTerminalReportRecorded {
            trial_id,
            run_index,
            report: Box::new(receipt.report().clone()),
            base_class: receipt.class(),
            expected_receipt: Box::new(receipt.clone()),
        });
        self.push(JournalEvent::RunCommitted {
            trial_id,
            run_index,
            receipt: Box::new(receipt.clone()),
        });
    }

    fn push(&mut self, event: JournalEvent) -> AuthenticatedJournalRecord {
        let sequence = u64::try_from(self.records.len()).expect("journal sequence");
        let entry = JournalEntry {
            schema_version: 7,
            sequence,
            previous: self.records.last().map(|record| record.entry_digest),
            session: self.session.clone(),
            event,
        };
        let entry_digest = digest::document("journal entry", &entry).expect("journal entry digest");
        let record = AuthenticatedJournalRecord {
            entry,
            entry_digest,
        };
        self.records.push(record.clone());
        record
    }
}

pub(super) fn rewrite_hidden_attempt(
    evidence: &mut CampaignEvidence,
    role: AttemptRole,
    proof: &AuthenticatedEvaluationProof,
) {
    let chain = &mut evidence.journal.authority.journal_chain;
    let start = chain
        .iter()
        .position(|record| {
            matches!(
                record.entry.event,
                JournalEvent::AttemptPrepared { role: saved, .. } if saved == role
            )
        })
        .expect("hidden attempt in journal chain");
    let old_trial = match chain[start].entry.event {
        JournalEvent::AttemptPrepared { trial_id, .. } => trial_id,
        _ => unreachable!("attempt search returned a different event"),
    };
    let end = chain
        .iter()
        .enumerate()
        .skip(start + 1)
        .find_map(|(index, record)| {
            matches!(
                record.entry.event,
                JournalEvent::CleanupRecorded { trial_id, .. } if trial_id == old_trial
            )
            .then_some(index)
        })
        .expect("hidden attempt cleanup in journal chain");
    let session = evidence.journal.head.entry.session.clone();
    let replacement = attempt_events(proof)
        .into_iter()
        .map(|event| unlinked_record(&session, event));
    chain.splice(start..=end, replacement);
}

fn attempt_events(proof: &AuthenticatedEvaluationProof) -> Vec<JournalEvent> {
    let mut events = vec![JournalEvent::AttemptPrepared {
        trial_id: proof.trial_id,
        role: proof.role,
        candidate: proof.candidate_digest,
        plan_digest: proof.plan_digest,
        transition: None,
    }];
    for (index, receipt) in proof.terminal_receipts.iter().enumerate() {
        let run_index = u64::try_from(index).expect("run index");
        events.extend(terminal_events(proof.trial_id, run_index, receipt));
    }
    events.push(JournalEvent::AttemptCompleted {
        trial_id: proof.trial_id,
        evaluation: proof.evaluation.clone(),
        proof: Some(Box::new(proof.clone())),
        selected_as_training_incumbent: None,
    });
    events.push(JournalEvent::CleanupRecorded {
        trial_id: proof.trial_id,
        cleanup: OperationStatus::Succeeded,
    });
    events
}

fn terminal_events(
    trial_id: u64,
    run_index: u64,
    receipt: &RunTerminalReceipt,
) -> Vec<JournalEvent> {
    vec![
        JournalEvent::RunPrepared {
            trial_id,
            run_index,
            context: receipt.context().clone(),
            run_intent_digest: receipt.intent().run_intent_digest(),
        },
        JournalEvent::RunBound {
            trial_id,
            run_index,
            terminal_plan: receipt.report().plan().clone(),
            binding: receipt.binding().clone(),
        },
        JournalEvent::RunTerminalIntentPrepared {
            trial_id,
            run_index,
            intent: receipt.intent().clone(),
        },
        JournalEvent::RunTerminalReportRecorded {
            trial_id,
            run_index,
            report: Box::new(receipt.report().clone()),
            base_class: receipt.class(),
            expected_receipt: Box::new(receipt.clone()),
        },
        JournalEvent::RunCommitted {
            trial_id,
            run_index,
            receipt: Box::new(receipt.clone()),
        },
    ]
}

fn unlinked_record(session: &SessionIdentity, event: JournalEvent) -> AuthenticatedJournalRecord {
    let entry = JournalEntry {
        schema_version: 7,
        sequence: 0,
        previous: None,
        session: session.clone(),
        event,
    };
    let entry_digest = digest::document("journal entry", &entry).expect("journal entry digest");
    AuthenticatedJournalRecord {
        entry,
        entry_digest,
    }
}

pub(super) fn rewrite_promotion_authority(
    evidence: &mut CampaignEvidence,
    frozen_candidate: Digest,
) {
    for record in &mut evidence.journal.authority.journal_chain {
        match &mut record.entry.event {
            JournalEvent::Frozen { candidate, .. } => *candidate = frozen_candidate,
            JournalEvent::PromotionClosed { closure } => {
                *closure = evidence.journal.promotion_closure.clone();
            }
            JournalEvent::Sealed {
                candidate,
                promotion_closure_digest,
                final_evaluation_digest,
                final_proof_digest,
                ..
            } => {
                let proof = evidence.journal.final_proof.as_ref().expect("final proof");
                *candidate = frozen_candidate;
                *promotion_closure_digest = evidence.journal.promotion_closure.closure_digest;
                *final_evaluation_digest = proof.evaluation_digest;
                *final_proof_digest = proof.proof_digest;
            }
            _ => {}
        }
    }
    evidence.journal.authority.frozen_candidate = frozen_candidate;
    evidence.journal.authority.final_candidate = Some(frozen_candidate);
}

pub(super) fn rechain_journal_authority(evidence: &mut CampaignEvidence) {
    let mut previous = None;
    for (index, record) in evidence
        .journal
        .authority
        .journal_chain
        .iter_mut()
        .enumerate()
    {
        record.entry.sequence = u64::try_from(index).expect("journal sequence");
        record.entry.previous = previous;
        record.entry_digest =
            digest::document("journal entry", &record.entry).expect("journal entry digest");
        previous = Some(record.entry_digest);
    }
    sync_named_records(evidence);
    sync_attempt_projection(evidence);
    let head = evidence
        .journal
        .authority
        .journal_chain
        .last()
        .expect("journal head");
    evidence.journal.head = AuthenticatedJournalHead {
        entry: head.entry.clone(),
        entry_digest: head.entry_digest,
    };
}

fn sync_named_records(evidence: &mut CampaignEvidence) {
    let chain = &evidence.journal.authority.journal_chain;
    evidence.journal.authority.frozen =
        find_record(chain, |event| matches!(event, JournalEvent::Frozen { .. }));
    evidence.journal.authority.promotion_baseline =
        find_attempt(chain, AttemptRole::PromotionBaseline);
    evidence.journal.authority.promotion_frozen =
        Some(find_attempt(chain, AttemptRole::PromotionFrozen));
    evidence.journal.authority.final_qualification =
        Some(find_attempt(chain, AttemptRole::FinalQualification));
}

/// Re-derives the projection after a test rewrites the chain.
///
/// A tamper test that leaves a stale projection behind would be caught by the
/// projection check rather than by the relation it means to attack.
fn sync_attempt_projection(evidence: &mut CampaignEvidence) {
    let limit = evidence.journal.stage.execution_retry.execution_retry_limit;
    if let Ok(projection) =
        AttemptProjection::from_journal_chain(&evidence.journal.authority.journal_chain, limit)
    {
        evidence.journal.authority.attempts = projection;
    }
}

fn find_attempt(
    chain: &[AuthenticatedJournalRecord],
    role: AttemptRole,
) -> AuthenticatedJournalRecord {
    find_record(chain, |event| {
        matches!(
            event,
            JournalEvent::AttemptPrepared { role: saved, .. } if *saved == role
        )
    })
}

fn find_record(
    chain: &[AuthenticatedJournalRecord],
    predicate: impl Fn(&JournalEvent) -> bool,
) -> AuthenticatedJournalRecord {
    chain
        .iter()
        .find(|record| predicate(&record.entry.event))
        .cloned()
        .expect("named journal record")
}

pub(super) fn assert_journal_chain_linked(evidence: &CampaignEvidence) {
    let mut previous = None;
    for (index, record) in evidence.journal.authority.journal_chain.iter().enumerate() {
        assert_eq!(
            record.entry.sequence,
            u64::try_from(index).expect("sequence")
        );
        assert_eq!(record.entry.previous, previous);
        assert_eq!(
            record.entry_digest,
            digest::document("journal entry", &record.entry).expect("entry digest")
        );
        previous = Some(record.entry_digest);
    }
    let head = evidence
        .journal
        .authority
        .journal_chain
        .last()
        .expect("journal head");
    assert_eq!(head.entry, evidence.journal.head.entry);
    assert_eq!(head.entry_digest, evidence.journal.head.entry_digest);
}
