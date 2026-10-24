use navigate_contract::{ClockDomainId, GeodeticPosition};
use pilotage_protocol::wire;

use super::{
    FrameAssessment, IntentContext, Lane, LaneConfig, LaneEffect, RunPlan, RunProgress,
    SurveyDefaults,
};
use crate::config::{MissionConfig, OwnshipSource};
use crate::lane::proposal_digest;

const NOW: u64 = 1_800_000_000_000;

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
        ..wire::MissionApproval::default()
    }
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
    let proposal = reply.receipt.proposal.expect("a proposal");
    let digest = proposal.digest.clone();
    let lines = proposal
        .tasks
        .iter()
        .filter(|t| t.task_id.starts_with("line"))
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
    let proposal = reply.receipt.proposal.expect("a proposal");
    assert_eq!(proposal.digest, proposal_digest(&proposal).to_vec());
    let estimates = proposal.estimates.expect("estimates");
    assert!(estimates.lines >= 2 && estimates.path_length_m > 300.0);
    assert!(estimates.energy_wh > 0.0);
    assert_eq!(
        proposal.tasks.last().map(|t| t.task_id.as_str()),
        Some("home")
    );
    let mut changed = proposal.clone();
    changed.tasks.pop();
    assert_ne!(proposal_digest(&changed).to_vec(), proposal.digest);
    assert_eq!(lane.state(), wire::MissionLaneState::Proposed);
}

#[test]
fn nothing_starts_without_an_approval_that_covers_the_plan() {
    use wire::mission_intent::Action;
    let mut lane = Lane::new(config());
    let digest = lane
        .on_intent(&intent(Action::Propose(objective())), &context("p"))
        .receipt
        .proposal
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
    lane.on_progress(&RunProgress {
        target_waypoint: Some(0),
        ..RunProgress::default()
    });
    assert!(lane.on_frame().is_none());
    // Along line 1: three pictures, two judged usable.
    lane.on_progress(&RunProgress {
        target_waypoint: Some(1),
        ..RunProgress::default()
    });
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
    lane.on_progress(&RunProgress {
        target_waypoint: Some(3),
        ..RunProgress::default()
    });
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
        target_waypoint: Some(2 * lines),
        ..RunProgress::default()
    });
    lane.on_progress(&RunProgress {
        target_waypoint: Some(2 * lines),
        plan_complete: true,
        ended: false,
    });
    let status = lane.take_changed().expect("changed");
    assert_eq!(status.state, wire::MissionLaneState::Complete as i32);
    let state = |id: &str| {
        status
            .tasks
            .iter()
            .find(|t| t.task_id == id)
            .map(|t| t.state)
    };
    assert_eq!(state("line-01"), Some(wire::MissionTaskState::Done as i32));
    assert_eq!(
        state("line-02"),
        Some(wire::MissionTaskState::Incomplete as i32)
    );
    assert_eq!(state("home"), Some(wire::MissionTaskState::Done as i32));
    let line2 = status
        .tasks
        .iter()
        .find(|t| t.task_id == "line-02")
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

mod evidence;
mod successor;
