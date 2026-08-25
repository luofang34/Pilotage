use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};

use super::rig::{FakeHandle, ObservedViews};
use super::{TestDirectory, TestTuner};
use crate::{CampaignPhase, JournalEntry};

#[derive(Debug, Clone, PartialEq)]
pub(super) struct EvidenceSnapshot {
    actions: ActionSnapshot,
    entries: Vec<JournalEntry>,
    phase: CampaignPhase,
    training_attempt_count: u64,
    head_bytes: Vec<u8>,
    catalog: BTreeMap<PathBuf, CatalogEntry>,
}

impl EvidenceSnapshot {
    pub(super) fn new(
        tuner: &TestTuner,
        directory: &TestDirectory,
        state: &FakeHandle,
        proposals: &ObservedViews,
    ) -> Self {
        Self {
            actions: ActionSnapshot::new(state, proposals),
            entries: tuner.journal().entries().to_vec(),
            phase: tuner.journal().phase(),
            training_attempt_count: tuner.journal().training_attempt_count(),
            head_bytes: fs::read(directory.path().join("HEAD.json")).expect("read journal head"),
            catalog: catalog(directory.path()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CatalogEntry {
    kind: CatalogKind,
    mode: u32,
    link_count: u64,
    bytes: Option<Vec<u8>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CatalogKind {
    Directory,
    File,
    Symlink,
    Other,
}

fn catalog(root: &Path) -> BTreeMap<PathBuf, CatalogEntry> {
    let mut entries = BTreeMap::new();
    collect_catalog(root, root, &mut entries);
    entries
}

fn collect_catalog(root: &Path, path: &Path, entries: &mut BTreeMap<PathBuf, CatalogEntry>) {
    let metadata = fs::symlink_metadata(path).expect("inspect evidence object");
    let relative = path.strip_prefix(root).expect("relative evidence path");
    let bytes = metadata
        .is_file()
        .then(|| fs::read(path).expect("read evidence object"));
    entries.insert(
        relative.to_path_buf(),
        CatalogEntry {
            kind: catalog_kind(&metadata),
            mode: metadata.permissions().mode() & 0o777,
            link_count: metadata.nlink(),
            bytes,
        },
    );
    if metadata.is_dir() {
        let mut children = fs::read_dir(path)
            .expect("read evidence directory")
            .map(|entry| entry.expect("read evidence entry").path())
            .collect::<Vec<_>>();
        children.sort();
        for child in children {
            collect_catalog(root, &child, entries);
        }
    }
}

fn catalog_kind(metadata: &fs::Metadata) -> CatalogKind {
    let kind = metadata.file_type();
    if kind.is_dir() {
        CatalogKind::Directory
    } else if kind.is_file() {
        CatalogKind::File
    } else if kind.is_symlink() {
        CatalogKind::Symlink
    } else {
        CatalogKind::Other
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ActionSnapshot {
    backend: [usize; 7],
    vehicle: [usize; 3],
    gates: [usize; 4],
    metric: [usize; 4],
    strategy_proposals: usize,
    lifecycle: Vec<String>,
}

impl ActionSnapshot {
    pub(super) fn new(state: &FakeHandle, proposals: &ObservedViews) -> Self {
        let state = state.0.borrow();
        Self {
            backend: [
                state.open_session_count,
                state.prepare_count,
                state.start_count,
                state.sample_poll_count,
                state.sample_count,
                state.stop_count,
                state.cleanup_count,
            ],
            vehicle: [
                state.vehicle.bind_count,
                state.vehicle.ensure_count,
                state.vehicle.apply_count,
            ],
            gates: [
                state.gate_begin_count,
                state.gate_evaluate_count,
                state.gate_finish_count,
                state.gate_cancel_count,
            ],
            metric: [
                state.metric_begin_count,
                state.metric_observe_count,
                state.metric_finish_count,
                state.metric_cancel_count,
            ],
            strategy_proposals: proposals.borrow().len(),
            lifecycle: state.lifecycle.clone(),
        }
    }
}

pub(super) fn completed_baseline_actions() -> ActionSnapshot {
    completed_actions(true)
}

pub(super) fn completed_without_final_cleanup_actions() -> ActionSnapshot {
    completed_actions(false)
}

fn completed_actions(final_cleanup: bool) -> ActionSnapshot {
    let mut lifecycle = vec!["open_session".to_owned(), "apply".to_owned()];
    append_terminal_run(&mut lifecycle);
    lifecycle.push("cleanup".to_owned());
    append_terminal_run(&mut lifecycle);
    if final_cleanup {
        lifecycle.push("cleanup".to_owned());
    }
    ActionSnapshot {
        backend: [1, 2, 2, 4, 2, 2, if final_cleanup { 2 } else { 1 }],
        vehicle: [1, 3, 1],
        gates: [2, 2, 2, 0],
        metric: [2, 2, 2, 0],
        strategy_proposals: 0,
        lifecycle,
    }
}

fn append_terminal_run(lifecycle: &mut Vec<String>) {
    lifecycle.extend(
        [
            "bind_terminal_plan",
            "prepare",
            "start",
            "sample",
            "stop",
            "terminal_control_stop",
            "terminal_trace_stop",
            "terminal_child_health",
            "terminal_trace_shutdown",
            "terminal_child_terminate",
            "read_causal_evidence",
            "recover_terminal_receipts",
            "seal_terminal_receipt",
            "recover_terminal_receipts",
        ]
        .map(str::to_owned),
    );
}
