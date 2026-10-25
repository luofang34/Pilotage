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

use pilotage_mission::executor::{
    AT_MOST, ExecutorRegistry, OperationDescriptor, READ_VALUE, WAIT_FOR_PERSON, WRITE_REPORT,
};
use pilotage_mission::{PlannedOperation, Reconciled, StatusQuery};
use pilotage_protocol::wire;
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

/// The operations a host configured as `host` carries out: the largest
/// set whose operations need only the capabilities the host has.
pub(crate) fn registry(host: &OperationHost) -> ExecutorRegistry {
    const SETS: &[&[OperationDescriptor]] = &[
        &[READ_VALUE, WAIT_FOR_PERSON, WRITE_REPORT, AT_MOST],
        &[READ_VALUE, WAIT_FOR_PERSON, AT_MOST],
        &[WAIT_FOR_PERSON, WRITE_REPORT, AT_MOST],
        &[WAIT_FOR_PERSON, AT_MOST],
    ];
    let has = host.capabilities();
    let fits = |set: &&&[OperationDescriptor]| {
        set.iter()
            .all(|d| d.capabilities.iter().all(|c| has.contains(c)))
    };
    ExecutorRegistry::new(SETS.iter().find(fits).copied().unwrap_or(&[]))
}

impl OperationHost {
    /// What the host can do: ask a person through its lane always, read
    /// values with a sources directory, store reports with a reports one.
    fn capabilities(&self) -> Vec<&'static str> {
        let mut has = vec!["person"];
        if self.sources.is_some() {
            has.push("data-source");
        }
        if self.reports.is_some() {
            has.push("report-store");
        }
        has
    }
}

/// What the run takes besides its plan: the results of its tasks that ended
/// before it started, and how many of its tasks the lane journal holds as
/// ended.
pub(crate) struct RunInputs {
    pub results: Vec<(String, Vec<wire::MissionValue>)>,
    pub recorded: watch::Receiver<usize>,
}

/// Spawns the run `run_id` of `plan` on `host`, steered by `controls`.
/// Returns the task handle and the status watch.
pub(crate) fn spawn_operation_run(
    run_id: String,
    plan: Vec<PlannedOperation>,
    host: OperationHost,
    controls: mpsc::Receiver<RunControl>,
    inputs: RunInputs,
) -> (JoinHandle<()>, watch::Receiver<AutomationStatus>) {
    let (status_tx, status_rx) = watch::channel(AutomationStatus::default());
    let run = runner::OperationRun::new(run_id, host, controls, status_tx, inputs);
    let handle = tokio::spawn(async move {
        run.run(plan).await;
        tracing::debug!(task = "operation-run", "task exited");
    });
    (handle, status_rx)
}

/// Whether the operation `query` names happened before the host restarted,
/// as its executor can tell.
///
/// # Errors
/// When the executor cannot tell, such as without its directory.
pub(crate) async fn status(
    host: &OperationHost,
    query: &StatusQuery,
) -> Result<Reconciled, String> {
    let operation = &query.operation;
    match (operation.executor.as_str(), operation.operation.as_str()) {
        ("host", "write-report") => {
            let reports = host
                .reports
                .clone()
                .ok_or("this host has no reports directory")?;
            let id = query.operation_id.clone();
            let found =
                tokio::task::spawn_blocking(move || executors::report_of_blocking(&reports, &id))
                    .await
                    .map_err(|join| format!("the status query stopped: {join}"))??;
            Ok(found.map_or(Reconciled::DidNotHappen, |report| {
                Reconciled::Happened(vec![wire::MissionValue {
                    name: "report".to_owned(),
                    value: report,
                }])
            }))
        }
        // A person's answer reaches only the run, which ended with the host.
        ("person", "confirm") => Ok(Reconciled::DidNotHappen),
        (executor, name) => Err(format!(
            "this host cannot tell whether {executor}/{name} happened"
        )),
    }
}
