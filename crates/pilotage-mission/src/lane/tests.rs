use navigate_contract::{ClockDomainId, GeodeticPosition};
use pilotage_protocol::wire;

use super::{
    FrameAssessment, IntentContext, Lane, LaneConfig, LaneEffect, RunPlan, RunProgress,
    SurveyDefaults,
};
use crate::config::{MissionConfig, OwnshipSource};

const NOW: u64 = 1_800_000_000_000;
/// The evidence window of the test lanes, milliseconds.
const WINDOW_MS: u64 = 60_000;

fn config() -> LaneConfig {
    let mut template = MissionConfig::new(
        String::new(),
        GeodeticPosition::new(0.0, 0.0, 0.0),
        ClockDomainId::new(1),
    );
    template.ownship_source = OwnshipSource::vehicle_estimate();
    LaneConfig {
        name: "SIM-1".to_owned(),
        simulated: true,
        execution_strategy: "host-guided-velocity".to_owned(),
        template,
        defaults: SurveyDefaults::default(),
        usable_energy_wh: Some(30.0),
        cruise_power_w: Some(150.0),
        recovery_policy: "hold in place".to_owned(),
        judged: true,
        evidence_window_ms: WINDOW_MS,
        executors: pilotage_mission_core::executor::ExecutorRegistry::NONE,
    }
}

fn point(latitude_deg: f64, longitude_deg: f64) -> wire::GeoPoint {
    wire::GeoPoint {
        latitude_deg,
        longitude_deg,
    }
}

/// About 330 m by 220 m south of Nassau Hall, flown at 60 m.
fn objective() -> wire::MissionObjective {
    wire::MissionObjective {
        objective_id: "objective-1".to_owned(),
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
                height_m: 60.0,
                cruise_mps: 10.0,
                return_to_launch: true,
                min_usable_frames_per_line: 2,
                ..wire::SurveyObjective::default()
            },
        )),
        ..wire::MissionObjective::default()
    }
}

fn intent(action: wire::mission_intent::Action) -> wire::MissionIntent {
    wire::MissionIntent {
        request_id: "request".to_owned(),
        action: Some(action),
    }
}

fn context(id: &str) -> IntentContext {
    IntentContext {
        now_unix_ms: NOW,
        fresh_id: id.to_owned(),
    }
}

fn approval(digest: &[u8], max_height_m: f64) -> wire::MissionApproval {
    wire::MissionApproval {
        approval_id: "approval-1".to_owned(),
        proposal_digest: digest.to_vec(),
        envelope: Some(wire::AdaptationEnvelope {
            max_height_m,
            min_reserve_fraction: 0.2,
            expires_unix_ms: NOW + 3_600_000,
            permit_execute: true,
            ..wire::AdaptationEnvelope::default()
        }),
        proposal_format: wire::MissionProposalFormat::V1 as i32,
        ..wire::MissionApproval::default()
    }
}

/// The proposal `record` carries, opened and verified.
fn open_record(record: wire::MissionProposalRecord) -> Option<wire::MissionProposal> {
    let sealed = pilotage_protocol::proposal_record::SealedProposal::open(record).ok()?;
    Some(sealed.into_parts().0)
}

/// The proposal a receipt's record carries, opened and verified.
fn opened(receipt: &wire::MissionReceipt) -> Option<wire::MissionProposal> {
    open_record(receipt.proposal_record.clone()?)
}

fn accepted(reply: &super::LaneReply) -> bool {
    reply.receipt.outcome == wire::MissionReceiptOutcome::Accepted as i32
}

/// Proposes, approves and starts; returns the lane and the line count.
fn running_lane() -> (Lane, usize) {
    running_lane_with(config())
}

/// [`running_lane`] under `config`.
fn running_lane_with(config: LaneConfig) -> (Lane, usize) {
    use wire::mission_intent::Action;
    let mut lane = Lane::new(config);
    let reply = lane.on_intent(
        &intent(Action::Propose(objective())),
        &context("proposal-1"),
    );
    let proposal = opened(&reply.receipt).expect("a proposal");
    let digest = proposal.digest.clone();
    let lines = proposal
        .tasks
        .iter()
        .filter(|t| t.kind == wire::MissionTaskKind::SurveyLine as i32)
        .count();
    assert!(accepted(&lane.on_intent(
        &intent(Action::Approve(approval(&digest, 120.0))),
        &context("a")
    )));
    let start = lane.on_intent(
        &intent(Action::Start(wire::MissionStart {
            proposal_digest: digest,
        })),
        &context("run-1"),
    );
    assert!(matches!(start.effect, Some(LaneEffect::Start(_))));
    (lane, lines)
}

#[test]
fn a_proposal_is_grounded_and_its_digest_binds_its_content() {
    use wire::mission_intent::Action;
    let mut lane = Lane::new(config());
    let reply = lane.on_intent(
        &intent(Action::Propose(objective())),
        &context("proposal-1"),
    );
    assert!(accepted(&reply), "{}", reply.receipt.reason);
    assert!(reply.effect.is_none(), "a proposal starts nothing");
    let proposal = opened(&reply.receipt).expect("a proposal");
    let record = reply.receipt.proposal_record.clone().expect("a record");
    assert_eq!(
        proposal.digest,
        pilotage_protocol::proposal_record::body_digest(&record.body).to_vec(),
        "the digest names the record's bytes"
    );
    assert!(proposal.requires.iter().any(|f| f.contains("survey-line")));
    let estimates = proposal.estimates.expect("estimates");
    assert!(estimates.lines >= 2 && estimates.path_length_m > 300.0);
    assert!(estimates.energy_wh > 0.0);
    assert_eq!(proposal.tasks.last().map(|t| t.name.as_str()), Some("home"));
    assert_eq!(lane.state(), wire::MissionLaneState::Proposed);
}

#[test]
fn nothing_starts_without_an_approval_that_covers_the_plan() {
    use wire::mission_intent::Action;
    let mut lane = Lane::new(config());
    let digest = lane
        .on_intent(&intent(Action::Propose(objective())), &context("p"))
        .receipt
        .proposal_record
        .and_then(open_record)
        .expect("proposal")
        .digest;
    let start = intent(Action::Start(wire::MissionStart {
        proposal_digest: digest.clone(),
    }));
    assert!(
        !accepted(&lane.on_intent(&start, &context("r"))),
        "no approval yet"
    );
    let low = lane.on_intent(
        &intent(Action::Approve(approval(&digest, 50.0))),
        &context("a"),
    );
    assert!(!accepted(&low) && low.receipt.reason.contains("envelope allows"));
    let other = lane.on_intent(
        &intent(Action::Approve(approval(b"other", 120.0))),
        &context("a"),
    );
    assert!(!accepted(&other));
    let mut expired = approval(&digest, 120.0);
    if let Some(envelope) = expired.envelope.as_mut() {
        envelope.expires_unix_ms = NOW;
    }
    assert!(!accepted(
        &lane.on_intent(&intent(Action::Approve(expired)), &context("a"))
    ));
    let mut bounded = approval(&digest, 120.0);
    if let Some(envelope) = bounded.envelope.as_mut() {
        envelope.bound = vec![point(40.0, -75.0), point(40.0, -74.9), point(40.1, -74.9)];
    }
    let outside = lane.on_intent(&intent(Action::Approve(bounded)), &context("a"));
    assert!(!accepted(&outside) && outside.receipt.reason.contains("leaves the approved bound"));
    assert!(accepted(&lane.on_intent(
        &intent(Action::Approve(approval(&digest, 120.0))),
        &context("a")
    )));
    assert!(accepted(&lane.on_intent(&start, &context("r"))));
    assert_eq!(lane.state(), wire::MissionLaneState::Running);
}

#[test]
fn a_line_is_done_only_with_usable_pictures() {
    let (mut lane, lines) = running_lane();
    assert!(lines >= 2);
    // Toward the start of line 1: a transit, no task takes pictures.
    lane.on_progress(&RunProgress { ..toward(&lane, 0) });
    assert!(lane.on_frame().is_none());
    // Along line 1: three pictures, two judged usable.
    lane.on_progress(&RunProgress { ..toward(&lane, 1) });
    for usable in [true, true, false] {
        let ticket = lane.on_frame().expect("line 1 is active");
        lane.on_sent(&ticket);
        lane.on_assessment(
            &ticket,
            Some(FrameAssessment {
                usable,
                hazard: false,
            }),
        )
        .unwrap();
    }
    // Along line 2: one picture, judged unusable.
    lane.on_progress(&RunProgress { ..toward(&lane, 3) });
    let ticket = lane.on_frame().expect("line 2 is active");
    lane.on_sent(&ticket);
    lane.on_assessment(
        &ticket,
        Some(FrameAssessment {
            usable: false,
            hazard: true,
        }),
    )
    .unwrap();
    lane.on_progress(&RunProgress {
        ..toward(&lane, 2 * lines)
    });
    lane.on_progress(&RunProgress {
        plan_complete: true,
        ended: false,
        ended_because: None,
        holding: false,
        execution: None,
        ..toward(&lane, 2 * lines)
    });
    let status = lane.take_changed().expect("changed");
    assert_eq!(status.state, wire::MissionLaneState::Complete as i32);
    let state = |id: &str| status.tasks.iter().find(|t| t.name == id).map(|t| t.state);
    assert_eq!(state("line-01"), Some(wire::MissionTaskState::Done as i32));
    assert_eq!(
        state("line-02"),
        Some(wire::MissionTaskState::Incomplete as i32)
    );
    assert_eq!(state("home"), Some(wire::MissionTaskState::Done as i32));
    let line2 = status
        .tasks
        .iter()
        .find(|t| t.name == "line-02")
        .and_then(|t| t.evidence.clone());
    assert_eq!(line2.map(|e| (e.frames, e.hazards)), Some((1, 1)));
}

#[test]
fn hold_resume_and_abort_follow_the_lifecycle() {
    use wire::mission_intent::Action;
    let (mut lane, _) = running_lane();
    let resume = lane.on_intent(
        &intent(Action::Resume(wire::MissionResume {})),
        &context("x"),
    );
    assert!(!accepted(&resume), "only a held run resumes");
    let hold = lane.on_intent(&intent(Action::Hold(wire::MissionHold {})), &context("x"));
    assert_eq!(hold.effect, Some(LaneEffect::Hold));
    assert_eq!(
        lane.state(),
        wire::MissionLaneState::HoldRequested,
        "held only once the vehicle says so"
    );
    lane.on_progress(&RunProgress::default());
    assert_eq!(lane.state(), wire::MissionLaneState::HoldRequested);
    lane.on_progress(&RunProgress {
        holding: true,
        execution: None,
        ..RunProgress::default()
    });
    assert_eq!(lane.state(), wire::MissionLaneState::Held);
    let resume = lane.on_intent(
        &intent(Action::Resume(wire::MissionResume {})),
        &context("x"),
    );
    assert_eq!(resume.effect, Some(LaneEffect::Resume));
    let propose = lane.on_intent(&intent(Action::Propose(objective())), &context("x"));
    assert!(!accepted(&propose), "a running lane takes no new objective");
    let abort = lane.on_intent(&intent(Action::Abort(wire::MissionAbort {})), &context("x"));
    assert_eq!(abort.effect, Some(LaneEffect::Abort));
    assert_eq!(lane.state(), wire::MissionLaneState::Aborted);
    let status = lane.take_changed().expect("changed");
    assert!(status.sequence > 0 && lane.take_changed().is_none());
}

fn task_states(lane: &Lane) -> Vec<(String, wire::MissionTaskState)> {
    lane.status()
        .tasks
        .iter()
        .filter_map(|t| {
            Some((
                t.name.clone(),
                wire::MissionTaskState::try_from(t.state).ok()?,
            ))
        })
        .collect()
}

#[test]
fn an_abort_leaves_the_line_in_progress_and_the_rest_unfinished() {
    use wire::mission_intent::Action;
    let (mut lane, lines) = running_lane();
    assert!(lines >= 2);
    // Along line 1 with enough pictures for its evidence: cut short, it
    // is still not done.
    lane.on_progress(&RunProgress { ..toward(&lane, 1) });
    for _ in 0..3 {
        let ticket = lane.on_frame().expect("line 1 is active");
        lane.on_sent(&ticket);
        lane.on_assessment(
            &ticket,
            Some(FrameAssessment {
                usable: true,
                hazard: false,
            }),
        )
        .unwrap();
    }
    lane.on_intent(&intent(Action::Abort(wire::MissionAbort {})), &context("x"));
    let states = task_states(&lane);
    assert!(
        states
            .iter()
            .all(|(_, s)| *s != wire::MissionTaskState::Done),
        "{states:?}"
    );
    assert!(states.contains(&("line-01".to_owned(), wire::MissionTaskState::Incomplete)));
    assert!(states.contains(&("line-02".to_owned(), wire::MissionTaskState::Pending)));
}

#[test]
fn an_abort_on_the_way_home_does_not_complete_the_way_home() {
    use wire::mission_intent::Action;
    let (mut lane, lines) = running_lane();
    // Every line flown; the vehicle flies home.
    lane.on_progress(&RunProgress {
        ..toward(&lane, 2 * lines + 1)
    });
    lane.on_intent(&intent(Action::Abort(wire::MissionAbort {})), &context("x"));
    let states = task_states(&lane);
    let home = states.iter().find(|(id, _)| !id.starts_with("line"));
    assert!(
        home.is_some_and(|(_, s)| *s == wire::MissionTaskState::Incomplete),
        "{states:?}"
    );
}

#[test]
fn a_run_reports_what_it_does_while_its_lane_flies() {
    let (mut lane, _) = running_lane();
    lane.on_progress(&RunProgress {
        execution: Some(super::RunExecution::LeaseHeldElsewhere(Some(7))),
        ..RunProgress::default()
    });
    let execution = lane.status().execution.expect("a run");
    assert_eq!(
        execution.state,
        wire::MissionExecutionState::LeaseHeldElsewhere as i32
    );
    assert_eq!(execution.lease_holder, 7);
    lane.on_intent(
        &intent(wire::mission_intent::Action::Abort(wire::MissionAbort {})),
        &context("x"),
    );
    assert_eq!(lane.status().execution, None, "no run, nothing to report");
}

/// What `lane`'s mission core reports while the vehicle flies toward waypoint `waypoint`
/// of the survey plan: line `k` is flown toward waypoint `2k + 1` after its transit toward
/// `2k`, and the way home follows the last line. The phases carry the lane's own task
/// identities.
pub(super) fn toward(lane: &Lane, waypoint: usize) -> RunProgress {
    let status = lane.status();
    let is_line = |id: &String| {
        status
            .tasks
            .iter()
            .any(|t| &t.task_id == id && t.kind == wire::MissionTaskKind::SurveyLine as i32)
    };
    let ids = lane.task_ids();
    let line_ids: Vec<String> = ids.iter().filter(|id| is_line(id)).cloned().collect();
    let lines = line_ids.len();
    if waypoint >= 2 * lines {
        return RunProgress {
            active_task: ids.iter().find(|id| !is_line(id)).cloned(),
            finished_tasks: line_ids,
            ..RunProgress::default()
        };
    }
    let flown = waypoint / 2;
    RunProgress {
        active_task: line_ids.get(flown).map(|id| {
            if waypoint % 2 == 1 {
                id.clone()
            } else {
                format!("transit:{id}")
            }
        }),
        finished_tasks: line_ids[..flown].to_vec(),
        ..RunProgress::default()
    }
}

mod admission;
mod approvals;
mod authorize;
mod evidence;
mod expiry;
mod fetch;
mod needs;
mod outcome;
mod report;
mod successor;
mod window;

#[test]
fn a_run_the_vehicle_ended_fails_with_its_reason() {
    let (mut lane, _) = running_lane();
    lane.on_progress(&RunProgress { ..toward(&lane, 0) });
    lane.on_progress(&RunProgress {
        ended: true,
        ended_because: Some("the vehicle landed outside the plan"),
        ..toward(&lane, 0)
    });
    let status = lane.take_changed().expect("changed");
    assert_eq!(status.state, wire::MissionLaneState::Failed as i32);
    assert_eq!(status.detail, "the vehicle landed outside the plan");
}

#[test]
fn tasks_follow_the_core_task_events_not_waypoint_indices() {
    let (mut lane, lines) = running_lane();
    assert!(lines >= 2);
    let ids = lane.task_ids();
    // However many waypoints a line has, the core names the task it flies.
    lane.on_progress(&RunProgress {
        active_task: Some(ids[1].clone()),
        finished_tasks: vec![format!("transit:{}", ids[0]), ids[0].clone()],
        ..RunProgress::default()
    });
    let states = task_states(&lane);
    assert!(states.contains(&("line-01".to_owned(), wire::MissionTaskState::Incomplete)));
    assert!(states.contains(&("line-02".to_owned(), wire::MissionTaskState::Active)));
    assert!(lane.on_frame().is_some(), "line 2 takes pictures");
    // A transit is no task: between lines nothing takes pictures.
    lane.on_progress(&RunProgress {
        active_task: Some(format!("transit:{}", ids[1])),
        finished_tasks: vec![ids[0].clone()],
        ..RunProgress::default()
    });
    assert!(lane.on_frame().is_none());
}
