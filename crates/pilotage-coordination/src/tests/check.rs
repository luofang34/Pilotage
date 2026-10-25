//! A kind of objective the team code does not name: a check goes to one
//! member whole and completes through the team's own lifecycle.

use pilotage_mission::executor::{
    AT_MOST, ExecutorRegistry, OperationDescriptor, READ_VALUE, WAIT_FOR_PERSON, WRITE_REPORT,
};
use pilotage_protocol::wire;
use wire::mission_intent::Action;

use super::*;

const HOST: &[OperationDescriptor] = &[READ_VALUE, WAIT_FOR_PERSON, WRITE_REPORT, AT_MOST];

#[test]
fn a_team_runs_a_check_through_one_member() {
    let mut fleet = Fleet::new();
    for (name, lane) in &mut fleet.members {
        *lane = member_lane(name, true, ExecutorRegistry::new(HOST));
    }
    let check = wire::MissionObjective {
        objective_id: "team-check".to_owned(),
        revision: 1,
        title: "Cabin temperature".to_owned(),
        kind: Some(wire::mission_objective::Kind::Check(wire::CheckObjective {
            source: "temperature".to_owned(),
            limit: "30".to_owned(),
        })),
        ..wire::MissionObjective::default()
    };
    let reply = fleet.team.on_intent(
        &wire::MissionIntent {
            request_id: "op-propose".to_owned(),
            action: Some(Action::Propose(check)),
        },
        &context("team-proposal"),
    );
    assert_eq!(reply.intents.len(), 1, "one member takes it whole");
    let member = reply.intents[0].member.clone();
    let proposal = fleet.route(reply.intents)[0]
        .proposal_record
        .clone()
        .and_then(open_record)
        .expect("the team proposal");
    fleet.approve_and_start(approval(&proposal.digest, 0));

    let lane = fleet.members.get_mut(&member).expect("the member");
    let ids = lane.task_ids();
    let value = |name: &str, value: &str| wire::MissionValue {
        name: name.to_owned(),
        value: value.to_owned(),
    };
    lane.on_progress(&RunProgress {
        finished_tasks: ids.clone(),
        results: vec![
            (ids[0].clone(), vec![value("value", "21.5 C")]),
            (
                ids[1].clone(),
                vec![
                    value("verdict", "21.5 C is within the limit of 30"),
                    value("within", "true"),
                ],
            ),
            (ids[2].clone(), vec![value("report", "run.txt")]),
        ],
        plan_complete: true,
        ..RunProgress::default()
    });
    let status = lane.take_changed().expect("changed");
    fleet.team.on_member_status(&status, &context("done"));
    let team = fleet.team.status();
    assert_eq!(
        team.state,
        wire::MissionLaneState::Complete as i32,
        "{}",
        team.detail
    );
}
