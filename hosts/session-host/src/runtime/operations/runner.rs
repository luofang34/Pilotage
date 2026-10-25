//! One run of a plan of operations: each task starts, ends with its
//! results, or ends the run. The status says which task is in progress,
//! which ended and with what, and what a person is asked now.

use std::collections::BTreeMap;

use pilotage_mission::PlannedOperation;
use pilotage_protocol::wire;
use sha2::{Digest, Sha256};
use tokio::sync::{mpsc, watch};
use tracing::{info, warn};

use super::OperationHost;
use super::executors::{read_value_blocking, write_report_blocking};
use crate::runtime::automation::{AutomationStatus, RunControl};

/// How one operation ended.
enum Step {
    /// It returned its results.
    Done(Vec<wire::MissionValue>),
    /// An abort stopped it.
    Aborted,
    /// It failed, for this reason.
    Failed(String),
}

pub(super) struct OperationRun {
    run_id: String,
    host: OperationHost,
    controls: mpsc::Receiver<RunControl>,
    status: watch::Sender<AutomationStatus>,
    now: AutomationStatus,
    /// The results of the tasks that ended, by task.
    results: BTreeMap<String, Vec<wire::MissionValue>>,
    /// An abort came while an operation that runs to its end ran.
    abort_after: bool,
}

impl OperationRun {
    pub(super) fn new(
        run_id: String,
        host: OperationHost,
        controls: mpsc::Receiver<RunControl>,
        status: watch::Sender<AutomationStatus>,
    ) -> Self {
        Self {
            run_id,
            host,
            controls,
            status,
            now: AutomationStatus::default(),
            results: BTreeMap::new(),
            abort_after: false,
        }
    }

    pub(super) async fn run(mut self, plan: Vec<PlannedOperation>) {
        for planned in plan {
            if !self.wait_while_held().await || self.abort_after {
                return self.stop();
            }
            let task_id = planned.task_id.clone();
            self.now.active_task = Some(task_id.clone());
            self.publish();
            let operation_id = format!("{}/{task_id}/1", self.run_id);
            match self.carry_out(&operation_id, &planned.operation).await {
                Step::Done(results) => {
                    let declined = results
                        .iter()
                        .any(|r| r.name == "confirmed" && r.value == "false");
                    self.finish(task_id, results);
                    if declined {
                        return self.end("the person declined");
                    }
                }
                Step::Aborted => return self.stop(),
                Step::Failed(reason) => {
                    warn!(operation = %operation_id, %reason, "operation failed");
                    return self.end("an operation failed");
                }
            }
        }
        if self.abort_after {
            return self.stop();
        }
        self.now.active_task = None;
        self.now.plan_complete = true;
        self.publish();
    }

    async fn carry_out(&mut self, operation_id: &str, operation: &wire::MissionOperation) -> Step {
        let parameters = match self.resolve(operation) {
            Ok(parameters) => parameters,
            Err(reason) => return Step::Failed(reason),
        };
        let given = |name: &str| parameters.get(name).cloned().unwrap_or_default();
        match (operation.executor.as_str(), operation.operation.as_str()) {
            ("host", "read-value") => {
                let Some(sources) = self.host.sources.clone() else {
                    return Step::Failed("this host has no sources directory".to_owned());
                };
                let source = given("source");
                self.blocking(move || read_value_blocking(&sources, &source), "value")
                    .await
            }
            ("host", "write-report") => {
                let Some(reports) = self.host.reports.clone() else {
                    return Step::Failed("this host has no reports directory".to_owned());
                };
                let (id, title, value) = (operation_id.to_owned(), given("title"), given("value"));
                // A report being written is written before an abort stops the run.
                let step = self
                    .blocking(
                        move || write_report_blocking(&reports, &id, &title, &value),
                        "report",
                    )
                    .await;
                self.note_abort();
                step
            }
            ("person", "confirm") => {
                self.confirm(operation_id, &given("question"), &given("subject"))
                    .await
            }
            (executor, name) => {
                Step::Failed(format!("this host has no operation {executor}/{name}"))
            }
        }
    }

    /// The parameters of `operation`, with each result of an earlier task
    /// it names.
    fn resolve(
        &self,
        operation: &wire::MissionOperation,
    ) -> Result<BTreeMap<String, String>, String> {
        operation
            .parameters
            .iter()
            .map(|p| {
                let value = match &p.source {
                    Some(wire::mission_parameter::Source::Value(value)) => value.clone(),
                    Some(wire::mission_parameter::Source::ResultOf(of)) => self
                        .results
                        .get(&of.task_id)
                        .and_then(|results| results.iter().find(|r| r.name == of.result))
                        .map(|r| r.value.clone())
                        .ok_or_else(|| format!("{} has no result {} yet", of.task_id, of.result))?,
                    None => return Err(format!("{} names no value", p.name)),
                };
                Ok((p.name.clone(), value))
            })
            .collect()
    }

    /// Runs `work` off the async threads; its value is the result `name`.
    async fn blocking(
        &mut self,
        work: impl FnOnce() -> Result<String, String> + Send + 'static,
        name: &str,
    ) -> Step {
        match tokio::task::spawn_blocking(work).await {
            Ok(Ok(value)) => Step::Done(vec![wire::MissionValue {
                name: name.to_owned(),
                value,
            }]),
            Ok(Err(reason)) => Step::Failed(reason),
            Err(join) => Step::Failed(format!("the operation stopped: {join}")),
        }
    }

    /// Asks a person `question` about `subject`, and waits for the answer
    /// the lane gives, or for an abort.
    async fn confirm(&mut self, operation_id: &str, question: &str, subject: &str) -> Step {
        let asked = wire::MissionQuestion {
            operation_id: operation_id.to_owned(),
            prompt: question.to_owned(),
            subject: subject.to_owned(),
            subject_digest: Sha256::digest(subject.as_bytes()).to_vec(),
        };
        if let Some(task) = self.now.active_task.clone() {
            self.now.questions = vec![(task, asked)];
        }
        self.publish();
        loop {
            match self.controls.recv().await {
                Some(RunControl::Answer(answer)) if answer.operation_id == operation_id => {
                    self.now.questions.clear();
                    info!(operation = %operation_id, accepted = answer.accepted, by = %answer.answered_by, "answered");
                    return Step::Done(vec![
                        wire::MissionValue {
                            name: "by".to_owned(),
                            value: answer.answered_by,
                        },
                        wire::MissionValue {
                            name: "confirmed".to_owned(),
                            value: answer.accepted.to_string(),
                        },
                    ]);
                }
                Some(RunControl::Abort) | None => {
                    self.now.questions.clear();
                    return Step::Aborted;
                }
                Some(control) => self.on_control(control),
            }
        }
    }

    /// Waits while a hold keeps the run from its next task. Returns `false`
    /// when an abort ends the run instead.
    async fn wait_while_held(&mut self) -> bool {
        loop {
            match self.controls.try_recv() {
                Ok(RunControl::Abort) | Err(mpsc::error::TryRecvError::Disconnected) => {
                    return false;
                }
                Ok(control) => self.on_control(control),
                Err(mpsc::error::TryRecvError::Empty) if !self.now.operator_held => return true,
                Err(mpsc::error::TryRecvError::Empty) => match self.controls.recv().await {
                    Some(RunControl::Abort) | None => return false,
                    Some(control) => self.on_control(control),
                },
            }
        }
    }

    /// Takes an abort that came while an operation ran to its end.
    fn note_abort(&mut self) {
        while let Ok(control) = self.controls.try_recv() {
            match control {
                RunControl::Abort => self.abort_after = true,
                control => self.on_control(control),
            }
        }
    }

    fn on_control(&mut self, control: RunControl) {
        match control {
            // Nothing moves between two operations, so a hold holds at once.
            RunControl::Hold => {
                self.now.operator_held = true;
                self.now.holding = true;
            }
            RunControl::Resume => {
                self.now.operator_held = false;
                self.now.holding = false;
            }
            RunControl::Abort => self.abort_after = true,
            // An answer for no question asked now is late; it changes nothing.
            RunControl::Answer(_) => {}
        }
        self.publish();
    }

    fn finish(&mut self, task_id: String, results: Vec<wire::MissionValue>) {
        self.now.finished_tasks.push(task_id.clone());
        self.now.results.push((task_id.clone(), results.clone()));
        self.results.insert(task_id, results);
        self.now.active_task = None;
        self.publish();
    }

    fn end(&mut self, because: &'static str) {
        self.now.ended_because = Some(because);
        self.now.stopped = true;
        self.publish();
    }

    fn stop(&mut self) {
        self.now.stopped = true;
        self.publish();
    }

    fn publish(&self) {
        self.status.send_replace(self.now.clone());
    }
}
