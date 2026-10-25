//! A run as its record keeps it, and as it is rebuilt from that record
//! after the host restarted: each task's execution and what its operation
//! returned. Evidence is not kept.

use pilotage_protocol::wire;
use wire::MissionTaskExecution as Execution;

use super::super::compile::Compiled;
use super::RunTracker;
use super::task::TaskTrack;

impl RunTracker {
    /// The run of `compiled` as `record` left it.
    pub(in crate::lane) fn restored(
        run_id: String,
        compiled: &Compiled,
        judged: bool,
        record: &[wire::MissionTaskRecord],
    ) -> Self {
        let mut run = Self::new(run_id, compiled, judged);
        for task in &mut run.tasks {
            if let Some(kept) = record.iter().find(|r| r.task_id == task.plan.task_id) {
                task.execution =
                    Execution::try_from(kept.execution).unwrap_or(Execution::Unspecified);
                task.results.clone_from(&kept.results);
            }
        }
        run
    }

    /// The tasks as the run's record keeps them.
    pub(in crate::lane) fn record(&self) -> Vec<wire::MissionTaskRecord> {
        self.tasks
            .iter()
            .map(|task| wire::MissionTaskRecord {
                task_id: task.plan.task_id.clone(),
                execution: task.execution as i32,
                results: task.results.clone(),
            })
            .collect()
    }

    /// The tasks of the run, in plan order, to change.
    pub(in crate::lane) fn tasks_mut(&mut self) -> impl Iterator<Item = &mut TaskTrack> {
        self.tasks.iter_mut()
    }

    /// The task `task_id`.
    pub(in crate::lane) fn task_mut(&mut self, task_id: &str) -> Option<&mut TaskTrack> {
        self.tasks
            .iter_mut()
            .find(|task| task.plan.task_id == task_id)
    }

    /// The tasks of the run, in plan order.
    pub(in crate::lane) fn tasks(&self) -> &[TaskTrack] {
        &self.tasks
    }
}
