use std::collections::BTreeMap;

use navigate_contract::{ClockDomainId, GeodeticPosition};
use pilotage_mission::{
    ExecutionTarget, FrameAssessment, IntentContext, Lane, LaneConfig, LaneEffect, MissionConfig,
    OwnshipSource, RunPlan, RunProgress, SurveyDefaults,
};
use pilotage_protocol::wire;

use crate::{Member, MemberIntent, TeamConfig, TeamLane, allocate};

const NOW: u64 = 1_800_000_000_000;

fn point(latitude_deg: f64, longitude_deg: f64) -> wire::GeoPoint {
    wire::GeoPoint {
        latitude_deg,
        longitude_deg,
    }
}

/// About 600 m by 400 m between the battlefield and the campus.
fn area() -> Vec<wire::GeoPoint> {
    vec![
        point(40.3330, -74.6720),
        point(40.3330, -74.6650),
        point(40.3366, -74.6650),
        point(40.3366, -74.6720),
    ]
}

fn defaults() -> SurveyDefaults {
    SurveyDefaults {
        height_m: 60.0,
        cruise_mps: 10.0,
        ..SurveyDefaults::default()
    }
}

fn vehicle_lane(name: &str) -> Lane {
    let mut template = MissionConfig::new(
        String::new(),
        GeodeticPosition::new(0.0, 0.0, 0.0),
        ClockDomainId::new(1),
    );
    template.ownship_source = OwnshipSource::vehicle_estimate();
    template.execution_target = ExecutionTarget::Simulator;
    Lane::new(LaneConfig {
        name: name.to_owned(),
        simulated: true,
        execution_strategy: "host-guided-velocity".to_owned(),
        template,
        defaults: defaults(),
        usable_energy_wh: None,
        cruise_power_w: None,
        recovery_policy: "hold".to_owned(),
        judged: true,
    })
}

fn context(id: &str) -> IntentContext {
    IntentContext {
        now_unix_ms: NOW,
        fresh_id: id.to_owned(),
    }
}

fn members() -> Vec<Member> {
    [
        ("SIM-1", point(40.3328, -74.6722)),
        ("SIM-2", point(40.3368, -74.6648)),
        ("SIM-3", point(40.3348, -74.6722)),
    ]
    .into_iter()
    .map(|(name, launch)| Member {
        name: name.to_owned(),
        launch,
        launch_altitude_m: 0.0,
    })
    .collect()
}

fn team() -> TeamLane {
    TeamLane::new(TeamConfig {
        name: "TEAM".to_owned(),
        members: members(),
        defaults: defaults(),
        recovery_policy: "hold".to_owned(),
    })
}

/// The member lanes, the routing between them and the team, and the runs
/// the members' hosts would fly.
struct Fleet {
    team: TeamLane,
    members: BTreeMap<String, Lane>,
    runs: BTreeMap<String, usize>,
    ids: u64,
}

impl Fleet {
    fn new() -> Self {
        let members = ["SIM-1", "SIM-2", "SIM-3"]
            .into_iter()
            .map(|n| (n.to_owned(), vehicle_lane(n)))
            .collect();
        Self {
            team: team(),
            members,
            runs: BTreeMap::new(),
            ids: 0,
        }
    }

    /// Delivers member intents and their receipts and statuses until the
    /// exchange settles. Returns the operator receipts the team decided.
    fn route(&mut self, mut intents: Vec<MemberIntent>) -> Vec<wire::MissionReceipt> {
        let mut decided = Vec::new();
        while let Some(MemberIntent { member, intent }) = intents.pop() {
            self.ids += 1;
            let ctx = context(&format!("id-{}", self.ids));
            let Some(lane) = self.members.get_mut(&member) else {
                continue;
            };
            let reply = lane.on_intent(&intent, &ctx);
            if let Some(LaneEffect::Start(order)) = reply.effect.as_ref() {
                let line_count = match &order.plan {
                    RunPlan::Survey(request) => request.polygon_deg.len().max(1),
                    RunPlan::Home => 0,
                };
                self.runs.insert(member.clone(), line_count);
            }
            let team_reply = self.team.on_member_receipt(&member, &reply.receipt, &ctx);
            decided.extend(team_reply.receipt);
            intents.extend(team_reply.intents);
            if let Some(status) = lane.take_changed() {
                intents.extend(self.team.on_member_status(&status, &ctx));
            }
        }
        decided
    }

    /// Approves the team proposal and starts it; both must be accepted.
    fn approve_and_start(&mut self, approval: wire::MissionApproval) {
        use wire::mission_intent::Action;
        let digest = approval.proposal_digest.clone();
        let intents = [
            Action::Approve(approval),
            Action::Start(wire::MissionStart {
                proposal_digest: digest,
            }),
        ];
        for (index, action) in intents.into_iter().enumerate() {
            let intent = wire::MissionIntent {
                request_id: format!("operator-{index}"),
                action: Some(action),
            };
            let reply = self.team.on_intent(&intent, &context("operator"));
            let decided = self.route(reply.intents);
            assert!(decided.first().is_some_and(accepted), "{decided:?}");
        }
    }

    /// Flies every running member's plan: each survey line gets three
    /// pictures, judged usable unless the line is in `bad`.
    fn fly(&mut self, bad: &[(&str, &str)]) -> Vec<MemberIntent> {
        let mut intents = Vec::new();
        let names: Vec<String> = self.members.keys().cloned().collect();
        for name in names {
            let Some(lane) = self.members.get_mut(&name) else {
                continue;
            };
            if lane.state() != wire::MissionLaneState::Running {
                continue;
            }
            let lines = lane
                .status()
                .tasks
                .iter()
                .filter(|t| t.kind == wire::MissionTaskKind::SurveyLine as i32)
                .count();
            for line in 0..lines {
                lane.on_progress(&RunProgress {
                    target_waypoint: Some(2 * line + 1),
                    ..RunProgress::default()
                });
                for _ in 0..3 {
                    if let Some(ticket) = lane.on_frame() {
                        let usable = !bad.contains(&(name.as_str(), ticket.task_id.as_str()));
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
                }
            }
            lane.on_progress(&RunProgress {
                target_waypoint: Some(2 * lines),
                plan_complete: true,
                ended: false,
                ended_because: None,
                holding: false,
                execution: None,
            });
            if let Some(status) = lane.take_changed() {
                intents.extend(self.team.on_member_status(&status, &context("fly")));
            }
        }
        intents
    }
}

fn accepted(receipt: &wire::MissionReceipt) -> bool {
    receipt.outcome == wire::MissionReceiptOutcome::Accepted as i32
}

fn approval(digest: &[u8], max_refly_lines: u32) -> wire::MissionApproval {
    wire::MissionApproval {
        approval_id: "team-approval".to_owned(),
        proposal_digest: digest.to_vec(),
        envelope: Some(wire::AdaptationEnvelope {
            max_height_m: 120.0,
            expires_unix_ms: NOW + 3_600_000,
            permit_execute: true,
            permit_revise: true,
            max_refly_lines,
            ..wire::AdaptationEnvelope::default()
        }),
        ..wire::MissionApproval::default()
    }
}

fn objective() -> wire::MissionObjective {
    wire::MissionObjective {
        objective_id: "team-objective".to_owned(),
        revision: 1,
        title: "battlefield".to_owned(),
        kind: Some(wire::mission_objective::Kind::Survey(
            wire::SurveyObjective {
                area: area(),
                launch: Some(point(40.3328, -74.6722)),
                min_usable_frames_per_line: 2,
                return_to_launch: true,
                ..wire::SurveyObjective::default()
            },
        )),
        ..wire::MissionObjective::default()
    }
}

#[test]
fn the_bands_cover_the_team_lines_once_each() {
    let members: Vec<(String, wire::GeoPoint)> = ["a", "b", "c"]
        .iter()
        .enumerate()
        .map(|(i, n)| ((*n).to_owned(), point(40.333 + 0.001 * i as f64, -74.672)))
        .collect();
    let assignments = allocate(&area(), &members, 90.0, 100.0, 40.0);
    let lines: Vec<usize> = assignments.iter().flat_map(|a| a.lines.clone()).collect();
    assert_eq!(
        lines,
        (1..=lines.len()).collect::<Vec<_>>(),
        "contiguous, each line once"
    );
    assert_eq!(assignments.len(), 3);
    assert!(assignments.iter().all(|a| a.band.len() >= 3));
}

#[test]
fn a_team_survey_is_proposed_approved_flown_and_adapted_inside_its_envelope() {
    use wire::mission_intent::Action;
    let mut fleet = Fleet::new();
    let propose = wire::MissionIntent {
        request_id: "op-1".to_owned(),
        action: Some(Action::Propose(objective())),
    };
    let reply = fleet.team.on_intent(&propose, &context("team-proposal"));
    assert!(
        reply.receipt.is_none(),
        "the team answers once its members proposed"
    );
    assert_eq!(reply.intents.len(), 3);
    let decided = fleet.route(reply.intents);
    let receipt = decided.first().expect("the team proposal");
    assert!(accepted(receipt), "{}", receipt.reason);
    let proposal = receipt.proposal.clone().expect("a team proposal");
    assert_eq!(proposal.members.len(), 3);
    let team_lines = proposal
        .tasks
        .iter()
        .filter(|t| t.kind == wire::MissionTaskKind::SurveyLine as i32)
        .count();
    assert!(team_lines >= 3);

    fleet.approve_and_start(approval(&proposal.digest, 2));
    assert_eq!(
        fleet.team.status().state,
        wire::MissionLaneState::Running as i32
    );

    // SIM-2 flies its first line with unusable pictures; the team flies it
    // again without asking the operator, with another member's camera, and
    // then completes.
    let refly = fleet.fly(&[("SIM-2", "line-01")]);
    assert_eq!(refly.len(), 1, "one re-flight");
    let flyer = refly.first().map(|i| i.member.clone()).unwrap_or_default();
    assert_ne!(flyer, "SIM-2", "not the camera that failed");
    fleet.route(refly);
    assert_eq!(
        fleet.members.get(&flyer).map(Lane::state),
        Some(wire::MissionLaneState::Running)
    );
    let more = fleet.fly(&[]);
    assert!(more.is_empty());
    let status = fleet.team.status();
    assert_eq!(
        status.state,
        wire::MissionLaneState::Complete as i32,
        "{}",
        status.detail
    );
    let line = status
        .tasks
        .iter()
        .find(|t| t.task_id == "SIM-2/line-01")
        .expect("the line");
    assert_eq!(
        (line.state, line.reflown),
        (wire::MissionTaskState::Done as i32, 1)
    );
    assert!(status.detail.contains("1 flown again"), "{}", status.detail);
    assert_eq!(status.members.len(), 3);
    assert!(!status.run_id.is_empty(), "a parent team follows this run");
}

#[test]
fn without_a_revise_permission_a_bad_line_stays_incomplete() {
    use wire::mission_intent::Action;
    let mut fleet = Fleet::new();
    let reply = fleet.team.on_intent(
        &wire::MissionIntent {
            request_id: "p".to_owned(),
            action: Some(Action::Propose(objective())),
        },
        &context("p"),
    );
    let proposal = fleet
        .route(reply.intents)
        .first()
        .and_then(|r| r.proposal.clone())
        .expect("proposal");
    let mut no_revise = approval(&proposal.digest, 2);
    if let Some(envelope) = no_revise.envelope.as_mut() {
        envelope.permit_revise = false;
    }
    let reply = fleet.team.on_intent(
        &wire::MissionIntent {
            request_id: "a".to_owned(),
            action: Some(Action::Approve(no_revise)),
        },
        &context("a"),
    );
    fleet.route(reply.intents);
    let reply = fleet.team.on_intent(
        &wire::MissionIntent {
            request_id: "s".to_owned(),
            action: Some(Action::Start(wire::MissionStart {
                proposal_digest: proposal.digest.clone(),
            })),
        },
        &context("s"),
    );
    fleet.route(reply.intents);
    assert!(
        fleet.fly(&[("SIM-1", "line-01")]).is_empty(),
        "no re-flight without permission"
    );
    let status = fleet.team.status();
    assert_eq!(status.state, wire::MissionLaneState::Complete as i32);
    let line = status
        .tasks
        .iter()
        .find(|t| t.task_id == "SIM-1/line-01")
        .expect("line");
    assert_eq!(line.state, wire::MissionTaskState::Incomplete as i32);
}

#[test]
fn an_idle_team_of_simulated_members_reports_simulated() {
    let mut fleet = Fleet::new();
    assert!(!fleet.team.status().simulated, "no member report yet");
    for (name, lane) in &fleet.members {
        let status = lane.status();
        assert!(status.simulated, "{name}");
        fleet.team.on_member_status(&status, &context("idle"));
    }
    let status = fleet.team.status();
    assert_eq!(status.state, wire::MissionLaneState::Idle as i32);
    assert!(status.simulated, "every member is simulated");
}

/// Delivers a parent team's intents to the fleet's team, and the team's
/// receipts and status back to the parent, until the exchange settles.
/// Returns the operator receipts the parent decided.
fn relay(
    parent: &mut TeamLane,
    fleet: &mut Fleet,
    mut intents: Vec<MemberIntent>,
) -> Vec<wire::MissionReceipt> {
    let mut decided = Vec::new();
    while let Some(MemberIntent { intent, .. }) = intents.pop() {
        let ctx = context("relay");
        let reply = fleet.team.on_intent(&intent, &ctx);
        let routed = fleet.route(reply.intents);
        for receipt in reply.receipt.into_iter().chain(routed) {
            let answer = parent.on_member_receipt("TEAM", &receipt, &ctx);
            decided.extend(answer.receipt);
            intents.extend(answer.intents);
        }
        if let Some(status) = fleet.team.take_changed() {
            intents.extend(parent.on_member_status(&status, &ctx));
        }
    }
    decided
}

#[test]
fn a_parent_team_counts_the_lines_its_member_team_flew_again() {
    use wire::mission_intent::Action;
    let mut fleet = Fleet::new();
    let mut parent = TeamLane::new(TeamConfig {
        name: "COMMAND".to_owned(),
        members: vec![Member {
            name: "TEAM".to_owned(),
            launch: point(40.3328, -74.6722),
            launch_altitude_m: 0.0,
        }],
        defaults: defaults(),
        recovery_policy: "hold".to_owned(),
    });
    let intent = |id: &str, action| wire::MissionIntent {
        request_id: id.to_owned(),
        action: Some(action),
    };
    let reply = parent.on_intent(&intent("p", Action::Propose(objective())), &context("p"));
    let proposal = relay(&mut parent, &mut fleet, reply.intents)
        .first()
        .and_then(|r| r.proposal.clone())
        .expect("the parent proposal");
    let approve = Action::Approve(approval(&proposal.digest, 2));
    let reply = parent.on_intent(&intent("a", approve), &context("a"));
    assert!(
        relay(&mut parent, &mut fleet, reply.intents)
            .first()
            .is_some_and(accepted)
    );
    let start = Action::Start(wire::MissionStart {
        proposal_digest: proposal.digest.clone(),
    });
    let reply = parent.on_intent(&intent("s", start), &context("s"));
    assert!(
        relay(&mut parent, &mut fleet, reply.intents)
            .first()
            .is_some_and(accepted)
    );
    for bad in [&[("SIM-2", "line-01")][..], &[]] {
        let refly = fleet.fly(bad);
        fleet.route(refly);
        if let Some(status) = fleet.team.take_changed() {
            parent.on_member_status(&status, &context("fly"));
        }
    }
    let status = parent.status();
    assert_eq!(
        status.state,
        wire::MissionLaneState::Complete as i32,
        "{}",
        status.detail
    );
    let line = status
        .tasks
        .iter()
        .find(|t| t.task_id == "TEAM/SIM-2/line-01")
        .expect("the nested line");
    assert_eq!(line.reflown, 1);
    assert!(status.detail.contains("1 flown again"), "{}", status.detail);
}

mod hold;
mod judge;
mod lines;
mod revision;
