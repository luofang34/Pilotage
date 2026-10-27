//! A run that waits for a hand-back is not replaced by a fresh one: the
//! fresh run's principal would take the lease as its first, without the
//! person's decision. The replacement, from an expiry's way home or from
//! a revision, starts only once control was handed back.

use std::sync::{Arc, Mutex};

use pilotage_protocol::proposal_record::SealedProposal;
use pilotage_protocol::wire;
use tokio::sync::{mpsc, watch};
use wire::mission_intent::Action;

use super::super::{ActiveRun, LaneTask, RunControl};
use super::restart::{host, lane, runtime, send};
use crate::runtime::AutomationStatus;
use pilotage_mission::{LaneEffect, MissionState};

const HOUR_MS: u64 = 3_600_000;

fn point(latitude_deg: f64, longitude_deg: f64) -> wire::GeoPoint {
    wire::GeoPoint {
        latitude_deg,
        longitude_deg,
    }
}

/// About 330 m by 220 m south of Nassau Hall, flown at `height_m`.
pub(super) fn survey(height_m: f64) -> wire::MissionObjective {
    wire::MissionObjective {
        objective_id: "survey-1".to_owned(),
        revision: 1,
        title: "test survey".to_owned(),
        kind: Some(wire::mission_objective::Kind::Survey(
            wire::SurveyObjective {
                area: vec![
                    point(40.3460, -74.6610),
                    point(40.3460, -74.6584),
                    point(40.3480, -74.6584),
                    point(40.3480, -74.6610),
                ],
                launch: Some(point(40.3459, -74.6611)),
                height_m,
                cruise_mps: 10.0,
                return_to_launch: true,
                min_usable_frames_per_line: 2,
                ..wire::SurveyObjective::default()
            },
        )),
        ..wire::MissionObjective::default()
    }
}

pub(super) fn approval(
    digest: &[u8],
    expires_unix_ms: u64,
    on_control_lost: wire::MissionControlLossPolicy,
) -> wire::MissionApproval {
    wire::MissionApproval {
        approval_id: format!("approval-{}", digest.first().copied().unwrap_or(0)),
        proposal_digest: digest.to_vec(),
        envelope: Some(wire::AdaptationEnvelope {
            max_height_m: 120.0,
            min_reserve_fraction: 0.2,
            expires_unix_ms,
            permit_execute: true,
            permit_revise: true,
            on_expiry: wire::MissionExpiryPolicy::ReturnHome as i32,
            on_control_lost: on_control_lost as i32,
            ..wire::AdaptationEnvelope::default()
        }),
        proposal_format: wire::MissionProposalFormat::V1 as i32,
        ..wire::MissionApproval::default()
    }
}

/// The digest of the lane's proposal under review, as a fetch returns it.
pub(super) fn fetched(task: &mut LaneTask, digest: Vec<u8>) -> Vec<u8> {
    let record = task
        .lane
        .on_intent(
            &wire::MissionIntent {
                request_id: "fetch".to_owned(),
                action: Some(Action::Fetch(wire::MissionFetch { digest })),
            },
            &pilotage_mission::IntentContext {
                now_unix_ms: 0,
                fresh_id: "fetch".to_owned(),
            },
        )
        .receipt
        .proposal_record
        .expect("the proposal record");
    SealedProposal::open(record)
        .expect("a proposal")
        .proposal()
        .digest
        .clone()
}

/// Starts a survey whose approval expires at `expires` and names the way
/// home, and replaces its run with a stand-in that lost the lease it held
/// to a person who released it without handing it back. Returns what the
/// stand-in reports and the controls it was sent.
pub(super) async fn flying_without_control(
    task: &mut LaneTask,
    expires: u64,
    on_control_lost: wire::MissionControlLossPolicy,
) -> (watch::Sender<AutomationStatus>, Arc<Mutex<Vec<RunControl>>>) {
    send(task, Action::Propose(survey(60.0))).await;
    let digest = fetched(task, task.lane.status().proposal_digest);
    send(
        task,
        Action::Approve(approval(&digest, expires, on_control_lost)),
    )
    .await;
    send(
        task,
        Action::Start(wire::MissionStart {
            proposal_digest: digest,
        }),
    )
    .await;
    let started = task.run.take().expect("the run started");
    started.handle.abort();
    let (status, receiver) = watch::channel(AutomationStatus {
        mission_state: Some(MissionState::Enroute),
        held: true,
        awaiting_hand_back: true,
        ..AutomationStatus::default()
    });
    let (controls, mut inbox) = mpsc::channel(8);
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = Arc::clone(&seen);
    let handle = tokio::spawn(async move {
        while let Some(control) = inbox.recv().await {
            let abort = control == RunControl::Abort;
            log.lock().expect("the log").push(control);
            if abort {
                return;
            }
        }
    });
    task.run = Some(ActiveRun {
        handle,
        status: receiver,
        controls,
        stopping: false,
    });
    (status, seen)
}

/// The person hands control back: the stand-in holds the lease again.
fn hand_back(status: &watch::Sender<AutomationStatus>) {
    status.send_modify(|s| {
        s.held = false;
        s.awaiting_hand_back = false;
        s.lease_generation = Some(8);
    });
}

async fn waits_then_starts(
    task: &mut LaneTask,
    status: &watch::Sender<AutomationStatus>,
    seen: &Arc<Mutex<Vec<RunControl>>>,
    effect: LaneEffect,
) {
    let runs = task.runs;
    task.apply(effect).await;
    assert!(task.after_hand_back.is_some(), "the replacement waits");
    assert_eq!(task.runs, runs, "no fresh run asks for the lease");
    task.on_progress().await;
    tokio::task::yield_now().await;
    assert_eq!(
        seen.lock().expect("the log").as_slice(),
        [RunControl::Hold],
        "the run that waits for the hand-back is held, not stopped"
    );
    assert!(task.after_hand_back.is_some(), "still no hand-back");
    // A resume of the lane does not resume the replaced run's plan.
    task.apply(LaneEffect::Resume).await;
    tokio::task::yield_now().await;
    assert_eq!(seen.lock().expect("the log").as_slice(), [RunControl::Hold]);
    hand_back(status);
    task.on_progress().await;
    assert!(task.after_hand_back.is_none(), "the replacement started");
    assert_eq!(task.runs, runs.wrapping_add(1));
    assert_eq!(
        seen.lock().expect("the log").as_slice(),
        [RunControl::Hold, RunControl::Abort],
        "the replaced run is stopped only after the hand-back"
    );
}

#[test]
fn an_expiry_way_home_waits_for_the_hand_back() {
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
        let effect = task
            .lane
            .on_clock(now + 2 * HOUR_MS)
            .expect("the way home replaces the run");
        assert!(matches!(effect, LaneEffect::Start(_)), "{effect:?}");
        waits_then_starts(&mut task, &status, &seen, effect).await;
    });
}

#[test]
fn a_revision_waits_for_the_hand_back() {
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
                    wire::MissionControlLossPolicy::Hold,
                ))),
            },
            &pilotage_mission::IntentContext {
                now_unix_ms: now,
                fresh_id: "approve-revision".to_owned(),
            },
        );
        let effect = reply.effect.expect("the revision replaces the run");
        assert!(matches!(effect, LaneEffect::Start(_)), "{effect:?}");
        waits_then_starts(&mut task, &status, &seen, effect).await;
    });
}
