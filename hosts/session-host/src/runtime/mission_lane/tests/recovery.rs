//! The approval's recovery at the host: a run that lost control may be
//! replaced by a run that only returns home once the taker's lease ended
//! without a hand-back, never while a person holds the lease, and never
//! after a revocation of the run's own lease.

use std::sync::{Arc, Mutex};

use pilotage_mission::LaneEffect;
use pilotage_protocol::wire;
use tokio::sync::watch;
use wire::mission_intent::Action;

use super::super::{LaneTask, RunControl};
use super::hand_back::{approval, flying_without_control, survey};
use super::restart::{host, lane, runtime, send};
use crate::runtime::AutomationStatus;

const HOUR_MS: u64 = 3_600_000;
const TAKER: u64 = 30;

/// A person took the lease from the run and holds it.
fn taken(status: &watch::Sender<AutomationStatus>) {
    status.send_modify(|s| {
        s.lease_holder = Some(TAKER);
        s.control_freed = false;
    });
}

/// The person's lease ended without a hand-back.
fn freed(status: &watch::Sender<AutomationStatus>) {
    status.send_modify(|s| {
        s.lease_holder = None;
        s.control_freed = true;
    });
}

fn stopped(seen: &Arc<Mutex<Vec<RunControl>>>) -> bool {
    seen.lock().expect("the log").contains(&RunControl::Abort)
}

async fn tick(task: &mut LaneTask) {
    task.on_progress().await;
    tokio::task::yield_now().await;
}

#[test]
fn a_freed_control_starts_the_approved_recovery_and_a_held_one_does_not() {
    let host = host();
    runtime().block_on(async {
        let (mut task, _published) = lane(&host).await;
        let now = super::super::unix_now_ms();
        let (status, seen) = flying_without_control(
            &mut task,
            now + HOUR_MS,
            wire::MissionControlLossPolicy::ReturnHome,
        )
        .await;
        let runs = task.runs;
        taken(&status);
        tick(&mut task).await;
        assert_eq!(task.runs, runs, "nothing moves against the person");
        assert!(!stopped(&seen));
        freed(&status);
        tick(&mut task).await;
        assert_eq!(task.runs, runs.wrapping_add(1), "the recovery starts");
        assert!(stopped(&seen), "the run that lost control is replaced");
        let detail = task.lane.status().detail;
        assert!(detail.contains("launch point"), "{detail}");
    });
}

#[test]
fn without_an_approved_recovery_a_freed_control_waits_for_a_hand_back() {
    let host = host();
    runtime().block_on(async {
        let (mut task, _published) = lane(&host).await;
        let now = super::super::unix_now_ms();
        let (status, seen) = flying_without_control(
            &mut task,
            now + HOUR_MS,
            wire::MissionControlLossPolicy::Hold,
        )
        .await;
        let runs = task.runs;
        taken(&status);
        tick(&mut task).await;
        freed(&status);
        tick(&mut task).await;
        assert_eq!(task.runs, runs);
        assert!(!stopped(&seen));
    });
}

#[test]
fn a_revocation_of_the_runs_own_lease_starts_no_recovery() {
    let host = host();
    runtime().block_on(async {
        let (mut task, _published) = lane(&host).await;
        let now = super::super::unix_now_ms();
        let (_status, seen) = flying_without_control(
            &mut task,
            now + HOUR_MS,
            wire::MissionControlLossPolicy::ReturnHome,
        )
        .await;
        // The stand-in lost its lease with no taker: nothing is freed.
        let runs = task.runs;
        tick(&mut task).await;
        tick(&mut task).await;
        assert_eq!(task.runs, runs, "a revocation is a stop");
        assert!(!stopped(&seen));
    });
}

#[test]
fn a_waiting_revision_gives_way_to_the_recovery_once_control_is_freed() {
    let host = host();
    runtime().block_on(async {
        let (mut task, _published) = lane(&host).await;
        let now = super::super::unix_now_ms();
        let (status, seen) = flying_without_control(
            &mut task,
            now + HOUR_MS,
            wire::MissionControlLossPolicy::ReturnHome,
        )
        .await;
        taken(&status);
        tick(&mut task).await;
        let state = task.lane.status();
        send(
            &mut task,
            Action::Revise(wire::MissionRevise {
                base_digest: state.proposal_digest.clone(),
                run_id: state.run_id.clone(),
                change: Some(wire::mission_revise::Change::Objective(survey(80.0))),
                ..wire::MissionRevise::default()
            }),
        )
        .await;
        let draft = task
            .lane
            .status()
            .successor
            .expect("a drafted revision")
            .digest;
        let reply = task.lane.on_intent(
            &wire::MissionIntent {
                request_id: "approve-revision".to_owned(),
                action: Some(Action::Approve(approval(
                    &draft,
                    now + HOUR_MS,
                    wire::MissionControlLossPolicy::ReturnHome,
                ))),
            },
            &pilotage_mission::IntentContext {
                now_unix_ms: now,
                fresh_id: "approve-revision".to_owned(),
            },
        );
        let runs = task.runs;
        task.apply(reply.effect.expect("the revision replaces the run"))
            .await;
        assert!(task.after_hand_back.is_some(), "the revised work waits");
        freed(&status);
        tick(&mut task).await;
        assert!(task.after_hand_back.is_none(), "the recovery replaced it");
        assert_eq!(task.runs, runs.wrapping_add(1));
        assert!(stopped(&seen));
        let detail = task.lane.status().detail;
        assert!(detail.contains("launch point"), "{detail}");
    });
}

#[test]
fn an_expiry_way_home_waits_while_a_person_holds_control_and_starts_once_freed() {
    let host = host();
    runtime().block_on(async {
        let (mut task, _published) = lane(&host).await;
        let now = super::super::unix_now_ms();
        let (status, seen) = flying_without_control(
            &mut task,
            now + HOUR_MS,
            wire::MissionControlLossPolicy::Hold,
        )
        .await;
        taken(&status);
        tick(&mut task).await;
        let effect = task
            .lane
            .on_clock(now + 2 * HOUR_MS)
            .expect("the way home replaces the run");
        assert!(matches!(&effect, LaneEffect::Start(order) if order.recovery));
        let runs = task.runs;
        task.apply(effect).await;
        assert!(task.after_hand_back.is_some(), "the person holds control");
        tick(&mut task).await;
        assert_eq!(task.runs, runs);
        freed(&status);
        tick(&mut task).await;
        assert!(task.after_hand_back.is_none());
        assert_eq!(task.runs, runs.wrapping_add(1), "the way home starts");
        assert!(stopped(&seen));
    });
}
