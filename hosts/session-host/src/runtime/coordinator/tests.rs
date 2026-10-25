use pilotage_coordination::{MemberIntent, TeamConfig, TeamLane};
use pilotage_protocol::wire;
use tokio::sync::mpsc;

use super::{TeamTask, outbox::Dispatch};

/// A coordinator task over no members but a link to `SIM-2`, writing its
/// journal to `journal`, whose journal last held one handover the lane no
/// longer has: the next send must write it first.
fn task(journal: std::path::PathBuf) -> (TeamTask, mpsc::Receiver<wire::MissionIntent>) {
    let lane = TeamLane::new(TeamConfig {
        name: "TEAM".to_owned(),
        members: Vec::new(),
        defaults: pilotage_mission::SurveyDefaults::default(),
        recovery_policy: "hold".to_owned(),
    });
    let (engine, _) = mpsc::channel(8);
    let (link, intents) = mpsc::channel(8);
    let earlier = wire::MissionHandover {
        assignment_id: "TEAM/run/handover-1".to_owned(),
        ..wire::MissionHandover::default()
    };
    let task = TeamTask {
        lane,
        journal,
        dispatch: Dispatch::new(vec![earlier]),
        engine,
        links: [("SIM-2".to_owned(), link)].into_iter().collect(),
        operator_requests: std::collections::BTreeMap::new(),
        ids: 0,
        last_publish: None,
        steering: pilotage_mission::Steering::default(),
    };
    (task, intents)
}

fn start_for_sim2() -> MemberIntent {
    MemberIntent {
        member: "SIM-2".to_owned(),
        intent: wire::MissionIntent {
            request_id: "team-start-1".to_owned(),
            action: Some(wire::mission_intent::Action::Start(
                wire::MissionStart::default(),
            )),
        },
    }
}

#[test]
fn work_held_while_the_journal_fails_goes_out_once_it_is_written() {
    let dir = tempfile::tempdir().unwrap();
    let journal = dir.path().join("not-yet").join("TEAM.handovers");
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    runtime.block_on(async {
        let (mut task, mut intents) = task(journal.clone());
        task.send(vec![start_for_sim2()]).await;
        assert!(
            intents.try_recv().is_err(),
            "nothing before its record is written"
        );
        assert!(task.dispatch.blocks_new_work());
        // The disk recovers: the held start goes out, once.
        std::fs::create_dir_all(journal.parent().unwrap()).unwrap();
        task.retry().await;
        assert_eq!(
            intents.try_recv().map(|i| i.request_id).ok().as_deref(),
            Some("team-start-1")
        );
        task.send(Vec::new()).await;
        assert!(intents.try_recv().is_err(), "sent once");
        assert!(!task.dispatch.blocks_new_work());
    });
}

#[test]
fn a_hold_sent_while_the_journal_fails_is_never_overtaken_by_older_work() {
    let dir = tempfile::tempdir().unwrap();
    let journal = dir.path().join("not-yet").join("TEAM.handovers");
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    runtime.block_on(async {
        let (mut task, mut intents) = task(journal.clone());
        task.send(vec![start_for_sim2()]).await;
        let hold = MemberIntent {
            member: "SIM-2".to_owned(),
            intent: wire::MissionIntent {
                request_id: "team-hold-1".to_owned(),
                action: Some(wire::mission_intent::Action::Hold(wire::MissionHold {})),
            },
        };
        task.send(vec![hold]).await;
        assert_eq!(
            intents.try_recv().map(|i| i.request_id).ok().as_deref(),
            Some("team-hold-1"),
            "the hold does not wait for the disk"
        );
        std::fs::create_dir_all(journal.parent().unwrap()).unwrap();
        task.retry().await;
        assert!(!task.dispatch.blocks_new_work());
        let after: Vec<String> = std::iter::from_fn(|| intents.try_recv().ok())
            .map(|i| i.request_id)
            .collect();
        assert_eq!(
            after,
            ["team-start-1", "team-hold-1"],
            "the held member gets its work and then the hold again"
        );
    });
}

mod wiring;
