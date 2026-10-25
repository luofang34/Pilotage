use pilotage_coordination::MemberIntent;
use pilotage_protocol::wire;
use wire::mission_intent::Action;

use super::{BACKLOG_LIMIT, Dispatch, Step, Withheld};

fn to(member: &str, action: Action) -> MemberIntent {
    MemberIntent {
        member: member.to_owned(),
        intent: wire::MissionIntent {
            request_id: format!("{member}-{}", action_name(&action)),
            action: Some(action),
        },
    }
}

fn action_name(action: &Action) -> &'static str {
    match action {
        Action::Hold(_) => "hold",
        Action::Abort(_) => "abort",
        Action::Start(_) => "start",
        _ => "other",
    }
}

fn start(member: &str) -> MemberIntent {
    to(member, Action::Start(wire::MissionStart::default()))
}

fn record(state: wire::MissionHandoverState) -> wire::MissionHandover {
    wire::MissionHandover {
        assignment_id: "TEAM/run/handover-1".to_owned(),
        state: state as i32,
        ..wire::MissionHandover::default()
    }
}

fn members(intents: &[MemberIntent]) -> Vec<String> {
    intents
        .iter()
        .map(|i| i.intent.request_id.clone())
        .collect()
}

#[test]
fn work_waits_for_the_journal_while_it_cannot_be_written() {
    let mut dispatch = Dispatch::new(Vec::new());
    let committed = vec![record(wire::MissionHandoverState::Committed)];
    let Step { send, write, .. } = dispatch.offer(vec![start("SIM-2")], committed.clone());
    assert!(send.is_empty(), "nothing before its record is written");
    let write = write.expect("the commit is written first");
    assert!(dispatch.written(write, false).is_empty());
    assert!(dispatch.blocks_new_work());
    assert!(
        dispatch.refusal().contains("1 intents wait"),
        "{}",
        dispatch.refusal()
    );
}

#[test]
fn stops_go_out_while_the_journal_fails() {
    let mut dispatch = Dispatch::new(Vec::new());
    let committed = vec![record(wire::MissionHandoverState::Committed)];
    let step = dispatch.offer(vec![start("SIM-2")], committed.clone());
    dispatch.written(step.write.expect("a write"), false);
    let stops = vec![
        to("SIM-1", Action::Hold(wire::MissionHold {})),
        to("SIM-2", Action::Abort(wire::MissionAbort {})),
    ];
    let step = dispatch.offer(stops, committed);
    assert_eq!(members(&step.send), ["SIM-1-hold", "SIM-2-abort"]);
}

#[test]
fn work_goes_out_once_when_the_journal_recovers() {
    let mut dispatch = Dispatch::new(Vec::new());
    let committed = vec![record(wire::MissionHandoverState::Committed)];
    let step = dispatch.offer(vec![start("SIM-2")], committed.clone());
    dispatch.written(step.write.clone().expect("a write"), false);
    let offered = dispatch.offer(Vec::new(), committed.clone());
    assert!(
        offered.write.is_none(),
        "a failing write waits for its retry"
    );
    let retry = dispatch
        .retry(committed.clone())
        .expect("the write is retried");
    let sent = dispatch.written(retry, true);
    assert_eq!(members(&sent), ["SIM-2-start"]);
    assert!(
        dispatch.retry(committed.clone()).is_none(),
        "nothing to retry"
    );
    assert!(!dispatch.blocks_new_work());
    let again = dispatch.offer(Vec::new(), committed);
    assert!(again.write.is_none(), "nothing left to write");
    assert!(again.send.is_empty(), "sent once");
}

fn failing(dispatch: &mut Dispatch, work: Vec<MemberIntent>) -> Vec<wire::MissionHandover> {
    let committed = vec![record(wire::MissionHandoverState::Committed)];
    let step = dispatch.offer(work, committed.clone());
    dispatch.written(step.write.expect("a write"), false);
    committed
}

fn requests(withheld: &[Withheld]) -> Vec<String> {
    withheld
        .iter()
        .map(|w| w.intent.intent.request_id.clone())
        .collect()
}

#[test]
fn a_hold_goes_out_at_once_and_waits_again_behind_its_members_work() {
    let mut dispatch = Dispatch::new(Vec::new());
    let committed = failing(&mut dispatch, vec![start("SIM-1"), start("SIM-2")]);
    let step = dispatch.offer(
        vec![to("SIM-2", Action::Hold(wire::MissionHold {}))],
        committed.clone(),
    );
    assert_eq!(
        members(&step.send),
        ["SIM-2-hold"],
        "the hold does not wait"
    );
    assert!(step.withheld.is_empty(), "a hold pauses work, it ends none");
    let sent = dispatch.written(dispatch.retry(committed).expect("a retry"), true);
    assert_eq!(
        members(&sent),
        ["SIM-1-start", "SIM-2-start", "SIM-2-hold"],
        "the member ends held, as with a written journal"
    );
}

#[test]
fn a_hold_for_a_member_with_no_waiting_work_does_not_wait_again() {
    let mut dispatch = Dispatch::new(Vec::new());
    let committed = failing(&mut dispatch, vec![start("SIM-1")]);
    let step = dispatch.offer(
        vec![to("SIM-2", Action::Hold(wire::MissionHold {}))],
        committed.clone(),
    );
    assert_eq!(members(&step.send), ["SIM-2-hold"]);
    let sent = dispatch.written(dispatch.retry(committed).expect("a retry"), true);
    assert_eq!(members(&sent), ["SIM-1-start"]);
}

#[test]
fn work_after_a_stop_in_one_answer_still_waits_for_the_journal() {
    let mut dispatch = Dispatch::new(Vec::new());
    let committed = failing(&mut dispatch, Vec::new());
    let step = dispatch.offer(
        vec![
            to("SIM-2", Action::Hold(wire::MissionHold {})),
            start("SIM-2"),
        ],
        committed.clone(),
    );
    assert_eq!(members(&step.send), ["SIM-2-hold"]);
    assert!(step.withheld.is_empty(), "the start came after the hold");
    let sent = dispatch.written(dispatch.retry(committed).expect("a retry"), true);
    assert_eq!(members(&sent), ["SIM-2-start"]);
}

#[test]
fn an_abort_for_a_member_withholds_its_waiting_work_and_the_hold_behind_it() {
    let mut dispatch = Dispatch::new(Vec::new());
    let committed = failing(&mut dispatch, vec![start("SIM-1"), start("SIM-2")]);
    dispatch.offer(
        vec![to("SIM-2", Action::Hold(wire::MissionHold {}))],
        committed.clone(),
    );
    let step = dispatch.offer(
        vec![to("SIM-2", Action::Abort(wire::MissionAbort {}))],
        committed.clone(),
    );
    assert_eq!(members(&step.send), ["SIM-2-abort"]);
    assert_eq!(
        requests(&step.withheld),
        ["SIM-2-start"],
        "the hold was sent"
    );
    let sent = dispatch.written(dispatch.retry(committed).expect("a retry"), true);
    assert_eq!(members(&sent), ["SIM-1-start"]);
}

#[test]
fn an_answer_whose_work_does_not_fit_is_withheld_whole() {
    let mut dispatch = Dispatch::new(Vec::new());
    let committed = failing(&mut dispatch, Vec::new());
    let mut filler: Vec<MemberIntent> = (1..BACKLOG_LIMIT).map(|_| start("SIM-1")).collect();
    assert!(
        dispatch
            .offer(std::mem::take(&mut filler), committed.clone())
            .withheld
            .is_empty()
    );
    let fan_out = vec![start("SIM-2"), start("SIM-3")];
    let step = dispatch.offer(fan_out, committed);
    assert_eq!(requests(&step.withheld), ["SIM-2-start", "SIM-3-start"]);
    assert!(
        step.withheld
            .iter()
            .all(|w| w.reason.contains("backlog is full"))
    );
    assert_eq!(dispatch.waiting(), BACKLOG_LIMIT - 1, "no part of it waits");
}

#[test]
fn work_whose_records_are_written_goes_out_without_a_write() {
    let committed = vec![record(wire::MissionHandoverState::Committed)];
    let mut dispatch = Dispatch::new(committed.clone());
    let step = dispatch.offer(vec![start("SIM-2")], committed);
    assert!(step.write.is_none());
    assert_eq!(members(&step.send), ["SIM-2-start"]);
}

#[test]
fn a_restart_at_any_point_of_a_handover_claims_no_outcome() {
    use pilotage_coordination::{TeamConfig, TeamLane};
    use wire::MissionHandoverState as State;
    // Before the commit is written the journal holds the open handover;
    // after it is written, sent or answered it holds the commit.
    for written in [State::Releasing, State::Committed] {
        let mut team = TeamLane::new(TeamConfig {
            name: "TEAM".to_owned(),
            members: Vec::new(),
            defaults: pilotage_mission::SurveyDefaults::default(),
            recovery_policy: "hold".to_owned(),
        });
        team.restore_handovers(&[record(written)]);
        let states: Vec<i32> = team.handovers().iter().map(|h| h.state).collect();
        assert_eq!(states, [State::Unreconciled as i32], "{written:?}");
    }
}
