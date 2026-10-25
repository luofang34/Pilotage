//! The host lane across a restart and a failing journal, on its real
//! paths: a report run is started, the host stops at a chosen point (its
//! run task is aborted and the lane task dropped, with no shutdown), and a
//! new lane task restores from the journal.

use std::path::{Path, PathBuf};
use std::time::Duration;

use pilotage_mission::{ExecutionTarget, LaneConfig, OwnshipSource, Requester, SurveyDefaults};
use pilotage_protocol::proposal_record::SealedProposal;
use pilotage_protocol::wire;
use pilotage_session::ClientKey;
use tokio::sync::mpsc;
use tokio::time::Instant;
use wire::MissionLaneState as State;
use wire::mission_intent::Action;

use super::super::{LaneRuntimeConfig, LaneTask};
use crate::runtime::automation::MissionPlan;
use crate::runtime::engine_actor::ToEngine;
use crate::runtime::operations::{self, OperationHost};

/// The directories of one host: what it reads, writes and keeps.
struct Host {
    _root: tempfile::TempDir,
    reports: PathBuf,
    journal: PathBuf,
    config: LaneRuntimeConfig,
}

fn host() -> Host {
    let root = tempfile::tempdir().expect("a directory");
    let sources = root.path().join("sources");
    let reports = root.path().join("reports");
    let state = root.path().join("state");
    for directory in [&sources, &reports, &state] {
        std::fs::create_dir(directory).expect("a directory");
    }
    std::fs::write(sources.join("temperature"), "21.5 C\n").expect("a source");
    let operations = OperationHost {
        sources: Some(sources),
        reports: Some(reports.clone()),
    };
    let journal = state.join("lane.journal");
    let config = LaneRuntimeConfig {
        lane: LaneConfig {
            name: "SIM-1".to_owned(),
            simulated: true,
            execution_strategy: "host-guided-velocity".to_owned(),
            template: MissionPlan::lane_template(
                OwnshipSource::vehicle_estimate(),
                ExecutionTarget::Simulator,
            ),
            defaults: SurveyDefaults::default(),
            usable_energy_wh: None,
            cruise_power_w: None,
            recovery_policy: "hold in place".to_owned(),
            judged: false,
            evidence_window_ms: 1_000,
            executors: operations::registry(&operations),
        },
        decision: None,
        assess_every: Duration::from_secs(6),
        judge_height_m: 0.0,
        planar_pose_is_truth: false,
        operations,
        journal: Some(journal.clone()),
    };
    Host {
        _root: root,
        reports,
        journal,
        config,
    }
}

/// A lane task of `host`, as its journal left it, and what it publishes.
async fn lane(host: &Host) -> (LaneTask, mpsc::Receiver<ToEngine>) {
    let (engine, published) = mpsc::channel(4096);
    let task = LaneTask::restored(host.config.clone(), engine, Instant::now())
        .await
        .expect("restored");
    (task, published)
}

fn operator() -> Requester {
    Requester {
        subject: "operator".to_owned(),
        may_approve: true,
    }
}

async fn send(task: &mut LaneTask, action: Action) {
    let mut intent = wire::MissionIntent {
        request_id: "request".to_owned(),
        action: Some(action),
    };
    assert!(
        task.on_intent(ClientKey::new(1), &operator(), &mut intent)
            .await
    );
}

/// Proposes, approves and starts the report of the `temperature` source.
async fn start_report(task: &mut LaneTask) {
    let objective = wire::MissionObjective {
        objective_id: "report-1".to_owned(),
        revision: 1,
        title: "Cabin temperature".to_owned(),
        kind: Some(wire::mission_objective::Kind::Report(
            wire::ReportObjective {
                source: "temperature".to_owned(),
                question: "Is this the cabin temperature?".to_owned(),
            },
        )),
        ..wire::MissionObjective::default()
    };
    send(task, Action::Propose(objective)).await;
    let record = task.lane.status().proposal_digest;
    let fetched = task
        .lane
        .on_intent(
            &wire::MissionIntent {
                request_id: "fetch".to_owned(),
                action: Some(Action::Fetch(wire::MissionFetch { digest: record })),
            },
            &pilotage_mission::IntentContext {
                now_unix_ms: 0,
                fresh_id: "fetch".to_owned(),
            },
        )
        .receipt
        .proposal_record
        .expect("the proposal record");
    let digest = SealedProposal::open(fetched)
        .expect("a proposal")
        .proposal()
        .digest
        .clone();
    let approval = wire::MissionApproval {
        approval_id: "approval-1".to_owned(),
        proposal_digest: digest.clone(),
        envelope: Some(wire::AdaptationEnvelope {
            max_height_m: 0.0,
            expires_unix_ms: u64::MAX,
            permit_execute: true,
            ..wire::AdaptationEnvelope::default()
        }),
        proposal_format: wire::MissionProposalFormat::V1 as i32,
        ..wire::MissionApproval::default()
    };
    send(task, Action::Approve(approval)).await;
    send(
        task,
        Action::Start(wire::MissionStart {
            proposal_digest: digest,
        }),
    )
    .await;
}

/// Folds each status of the run into the lane and records it, as the
/// host's clock does, until the status meets `until`.
async fn run_until(task: &mut LaneTask, until: impl Fn(&crate::runtime::AutomationStatus) -> bool) {
    let mut status = task.run.as_ref().expect("a run").status.clone();
    loop {
        // The status seen is marked first, so a change after it wakes the
        // wait below.
        let met = until(&status.borrow_and_update());
        task.on_progress().await;
        task.record().await;
        if met {
            return;
        }
        status.changed().await.expect("the run reports");
    }
}

/// The question the run asks now.
fn question(task: &LaneTask) -> wire::MissionQuestion {
    task.lane
        .status()
        .tasks
        .into_iter()
        .find_map(|t| t.question)
        .expect("a question")
}

async fn answer(task: &mut LaneTask, question: &wire::MissionQuestion) {
    send(
        task,
        Action::Answer(wire::MissionAnswer {
            operation_id: question.operation_id.clone(),
            subject_digest: question.subject_digest.clone(),
            accepted: true,
            answered_by: None,
            relayed_from: None,
        }),
    )
    .await;
}

/// The host stops at once: the run is not told to stop, and the lane task
/// ends without its shutdown.
fn crash(mut task: LaneTask) {
    if let Some(run) = task.run.take() {
        run.handle.abort();
    }
    drop(task);
}

fn reports(directory: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(directory)
        .expect("the reports")
        .map(|e| {
            e.expect("an entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    names.sort();
    names
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a runtime")
}

#[test]
fn a_restart_while_a_person_is_asked_holds_the_run_until_it_is_resumed() {
    let host = host();
    runtime().block_on(async {
        let (mut task, _published) = lane(&host).await;
        start_report(&mut task).await;
        run_until(&mut task, |s| !s.questions.is_empty()).await;
        let asked = question(&task);
        crash(task);

        let (mut task, _published) = lane(&host).await;
        assert_eq!(
            task.lane.state(),
            State::Held,
            "{}",
            task.lane.status().detail
        );
        assert_eq!(
            task.lane.status().holds,
            [wire::MissionHoldReason::Restarted as i32]
        );
        assert!(task.run.is_none(), "nothing runs again on its own");
        send(&mut task, Action::Resume(wire::MissionResume {})).await;
        run_until(&mut task, |s| !s.questions.is_empty()).await;
        let again = question(&task);
        assert_eq!(again.operation_id, asked.operation_id, "the same operation");
        answer(&mut task, &again).await;
        run_until(&mut task, |s| s.plan_complete).await;
        assert_eq!(task.lane.state(), State::Complete);
        assert_eq!(reports(&host.reports).len(), 1);
    });
}

#[test]
fn a_report_written_before_the_restart_is_found_and_not_written_again() {
    let host = host();
    runtime().block_on(async {
        let (mut task, _published) = lane(&host).await;
        start_report(&mut task).await;
        run_until(&mut task, |s| !s.questions.is_empty()).await;
        let asked = question(&task);
        answer(&mut task, &asked).await;
        run_until(&mut task, |s| s.finished_tasks.len() == 2).await;
        // The report is written, and the host stops before the lane hears.
        let mut status = task.run.as_ref().expect("a run").status.clone();
        status.wait_for(|s| s.plan_complete).await.expect("written");
        let written = reports(&host.reports);
        assert_eq!(written.len(), 1);
        let before = std::fs::read(host.reports.join(&written[0])).expect("the report");
        crash(task);

        let (task, _published) = lane(&host).await;
        assert_eq!(
            task.lane.state(),
            State::Complete,
            "{}",
            task.lane.status().detail
        );
        let write = task
            .lane
            .status()
            .tasks
            .into_iter()
            .last()
            .expect("the report task");
        assert_eq!(
            write.execution,
            wire::MissionTaskExecution::Completed as i32
        );
        assert_eq!(write.results[0].value, written[0], "its result is found");
        assert_eq!(reports(&host.reports), written);
        assert_eq!(
            std::fs::read(host.reports.join(&written[0])).expect("the report"),
            before
        );
    });
}

#[test]
fn a_cancelled_run_is_not_replayed_after_a_restart() {
    let host = host();
    runtime().block_on(async {
        let (mut task, _published) = lane(&host).await;
        start_report(&mut task).await;
        run_until(&mut task, |s| !s.questions.is_empty()).await;
        send(&mut task, Action::Abort(wire::MissionAbort {})).await;
        run_until(&mut task, |s| !s.cancelled_tasks.is_empty()).await;
        crash(task);

        let (mut task, _published) = lane(&host).await;
        assert_eq!(task.lane.state(), State::Aborted);
        send(&mut task, Action::Resume(wire::MissionResume {})).await;
        assert!(task.run.is_none());
        assert!(reports(&host.reports).is_empty());
    });
}

#[test]
fn no_run_starts_before_the_journal_holds_it_and_an_abort_still_passes() {
    let host = host();
    let state = host.journal.parent().expect("its directory").to_path_buf();
    runtime().block_on(async {
        let (mut task, _published) = lane(&host).await;
        std::fs::remove_dir_all(&state).expect("the journal's directory goes");
        start_report(&mut task).await;
        assert_eq!(task.lane.state(), State::Running);
        assert!(task.run.is_none(), "the run waits for the journal");
        std::fs::create_dir(&state).expect("the directory again");
        task.record().await;
        assert!(task.run.is_some(), "the run starts once it is recorded");

        // The journal fails again: a hold and a resume pass, and the run
        // begins no next operation until the journal holds it.
        run_until(&mut task, |s| !s.questions.is_empty()).await;
        std::fs::remove_dir_all(&state).expect("the journal's directory goes");
        send(&mut task, Action::Hold(wire::MissionHold {})).await;
        send(&mut task, Action::Resume(wire::MissionResume {})).await;
        let asked = question(&task);
        answer(&mut task, &asked).await;
        run_until(&mut task, |s| s.finished_tasks.len() == 2).await;
        assert!(
            reports(&host.reports).is_empty(),
            "no report while unrecorded"
        );
        std::fs::create_dir(&state).expect("the directory again");
        task.record().await;
        run_until(&mut task, |s| s.plan_complete).await;
        assert_eq!(reports(&host.reports).len(), 1);
        assert_eq!(task.lane.state(), State::Complete);
    });
}

#[test]
fn an_abort_drops_a_run_the_journal_never_held() {
    let host = host();
    let state = host.journal.parent().expect("its directory").to_path_buf();
    runtime().block_on(async {
        let (mut task, _published) = lane(&host).await;
        std::fs::remove_dir_all(&state).expect("the journal's directory goes");
        start_report(&mut task).await;
        send(&mut task, Action::Abort(wire::MissionAbort {})).await;
        assert_eq!(task.lane.state(), State::Aborted);
        std::fs::create_dir(&state).expect("the directory again");
        task.record().await;
        assert!(task.run.is_none(), "the aborted run never starts");
    });
}

#[test]
fn a_hold_while_the_run_waits_for_the_journal_reaches_the_run() {
    let host = host();
    let state = host.journal.parent().expect("its directory").to_path_buf();
    runtime().block_on(async {
        let (mut task, _published) = lane(&host).await;
        std::fs::remove_dir_all(&state).expect("the journal's directory goes");
        start_report(&mut task).await;
        send(&mut task, Action::Hold(wire::MissionHold {})).await;
        assert!(task.run.is_none(), "the run waits for the journal");
        std::fs::create_dir(&state).expect("the directory again");
        task.record().await;
        run_until(&mut task, |s| s.holding || !s.finished_tasks.is_empty()).await;
        let status = task.run.as_ref().expect("a run").status.borrow().clone();
        assert!(status.holding, "{status:?}");
        assert!(status.finished_tasks.is_empty(), "nothing ran: {status:?}");
        assert_eq!(task.lane.state(), State::Held);
    });
}
