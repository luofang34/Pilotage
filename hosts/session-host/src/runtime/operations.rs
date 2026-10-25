//! The host's executors and the run of a plan of operations (ADR-0041,
//! ADR-0052). A lane admits a plan only when this host registers each of
//! its operations in that version, so the registry follows what the host
//! is configured with: it reads values only from a sources directory and
//! writes reports only into a reports directory. A person answers through
//! the lane, so the person's operation is always registered.
//!
//! The run carries the operations out in order, one task each, and reports
//! on the status the flight run uses: the task in progress, the tasks it
//! finished with their results, and the question an operation asks. A
//! person who declines ends the run. A cancel follows each operation's
//! registered semantics: a read and a question stop at once; a report
//! being written is written, and reported, before the run stops.

mod executors;
mod runner;
#[cfg(test)]
mod tests;

use std::path::PathBuf;

use pilotage_mission::PlannedOperation;
use pilotage_mission::executor::{
    ExecutorRegistry, OperationDescriptor, READ_VALUE, WAIT_FOR_PERSON, WRITE_REPORT,
};
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;

use super::automation::{AutomationStatus, RunControl};

/// Where this host's executors read and write.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct OperationHost {
    /// The directory whose files `host/read-value` reads, by source name.
    pub sources: Option<PathBuf>,
    /// The directory `host/write-report` writes reports into.
    pub reports: Option<PathBuf>,
}

/// The operations a host configured as `host` carries out.
pub(crate) fn registry(host: &OperationHost) -> ExecutorRegistry {
    const ALL: &[OperationDescriptor] = &[READ_VALUE, WAIT_FOR_PERSON, WRITE_REPORT];
    const READS: &[OperationDescriptor] = &[READ_VALUE, WAIT_FOR_PERSON];
    const WRITES: &[OperationDescriptor] = &[WAIT_FOR_PERSON, WRITE_REPORT];
    const PERSON: &[OperationDescriptor] = &[WAIT_FOR_PERSON];
    ExecutorRegistry::new(match (host.sources.is_some(), host.reports.is_some()) {
        (true, true) => ALL,
        (true, false) => READS,
        (false, true) => WRITES,
        (false, false) => PERSON,
    })
}

/// Spawns the run `run_id` of `plan` on `host`, steered by `controls`.
/// Returns the task handle and the status watch.
pub(crate) fn spawn_operation_run(
    run_id: String,
    plan: Vec<PlannedOperation>,
    host: OperationHost,
    controls: mpsc::Receiver<RunControl>,
) -> (JoinHandle<()>, watch::Receiver<AutomationStatus>) {
    let (status_tx, status_rx) = watch::channel(AutomationStatus::default());
    let run = runner::OperationRun::new(run_id, host, controls, status_tx);
    let handle = tokio::spawn(async move {
        run.run(plan).await;
        tracing::debug!(task = "operation-run", "task exited");
    });
    (handle, status_rx)
}
