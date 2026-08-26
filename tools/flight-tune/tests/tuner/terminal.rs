#[path = "terminal/downstream.rs"]
mod downstream;
#[path = "terminal/evidence.rs"]
mod evidence;
#[path = "terminal/journal_authority.rs"]
mod journal_authority;
#[path = "terminal/operations.rs"]
mod operations;
#[path = "terminal/recovery.rs"]
mod recovery;

use std::fs;
use std::path::Path;

use flight_tune::{
    Digest, JournalEntry, JournalEvent, RunTerminalOperation, RunTerminalReceipt, TuneError,
};
use serde::Serialize;
use sha2::{Digest as ShaDigest, Sha256};

use super::test_rig::{FakeHandle, SequenceStrategy, TestDirectory, stage};
use super::{TestTuner, open_stage};

const ADAPTER_OPERATIONS_FOR_TEST: [RunTerminalOperation; 5] = [
    RunTerminalOperation::ControlStop,
    RunTerminalOperation::TraceStop,
    RunTerminalOperation::ChildHealth,
    RunTerminalOperation::TraceShutdown,
    RunTerminalOperation::ChildTerminate,
];

struct DurableRun {
    directory: TestDirectory,
    entries: Vec<JournalEntry>,
    receipt: RunTerminalReceipt,
}

impl DurableRun {
    fn rewind(&self, boundary: CrashBoundary) {
        let entry = self
            .entries
            .iter()
            .find(|entry| boundary.matches(&entry.event))
            .expect("terminal crash boundary");
        write_head(self.directory.path(), document_digest(entry));
    }
}

#[derive(Clone, Copy)]
enum CrashBoundary {
    RunPrepared,
    RunBound,
    IntentPrepared,
    ReportRecorded,
    RunCommitted,
}

impl CrashBoundary {
    const fn matches(self, event: &JournalEvent) -> bool {
        matches!(
            (self, event),
            (Self::RunPrepared, JournalEvent::RunPrepared { .. })
                | (Self::RunBound, JournalEvent::RunBound { .. })
                | (
                    Self::IntentPrepared,
                    JournalEvent::RunTerminalIntentPrepared { .. }
                )
                | (
                    Self::ReportRecorded,
                    JournalEvent::RunTerminalReportRecorded { .. }
                )
                | (Self::RunCommitted, JournalEvent::RunCommitted { .. })
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TerminalActions {
    simulator_stop: usize,
    bind: usize,
    operations: Vec<RunTerminalOperation>,
    causal_read: usize,
    seal: usize,
    recover: usize,
}

impl TerminalActions {
    fn capture(state: &FakeHandle) -> Self {
        let state = state.0.borrow();
        Self {
            simulator_stop: state.stop_count,
            bind: state.terminal.bind_count(),
            operations: state.terminal.operation_order().to_vec(),
            causal_read: state.terminal.causal_evidence_read_count(),
            seal: state.terminal.seal_count(),
            recover: state.terminal.recover_count(),
        }
    }
}

fn durable_run(label: &str, failed_operations: Vec<RunTerminalOperation>) -> DurableRun {
    let directory = TestDirectory::new(label);
    let state = FakeHandle::new();
    state.0.borrow_mut().terminal.failed_operations = failed_operations;
    let mut tuner = open_one(directory.path(), state).expect("open source run");
    let result = tuner.run_training_attempts_blocking(0);
    let entries = tuner.journal().entries().to_vec();
    let receipt = latest_receipt(&entries).clone();
    if !receipt.is_completed() {
        assert!(result.is_err());
    }
    drop(tuner);
    DurableRun {
        directory,
        entries,
        receipt,
    }
}

fn open_one(path: &Path, state: FakeHandle) -> Result<TestTuner, TuneError> {
    open_with_gate(path, state, -1.0)
}

fn open_with_gate(path: &Path, state: FakeHandle, gate_limit: f64) -> Result<TestTuner, TuneError> {
    open_stage(
        path,
        state,
        SequenceStrategy::new(Vec::new()),
        gate_limit,
        stage(),
    )
}

fn latest_receipt(entries: &[JournalEntry]) -> &RunTerminalReceipt {
    entries
        .iter()
        .rev()
        .find_map(|entry| match &entry.event {
            JournalEvent::RunCommitted { receipt, .. } => Some(receipt.as_ref()),
            _ => None,
        })
        .expect("committed terminal receipt")
}

fn document_digest(value: &impl Serialize) -> Digest {
    let bytes = serde_json::to_vec(value).expect("encode journal entry");
    Digest::from_bytes(Sha256::digest(bytes).into())
}

fn write_head(root: &Path, digest: Digest) {
    let document = serde_json::json!({ "digest": digest });
    let bytes = serde_json::to_vec(&document).expect("encode journal head");
    fs::write(root.join("HEAD.json"), bytes).expect("set terminal crash boundary");
}
