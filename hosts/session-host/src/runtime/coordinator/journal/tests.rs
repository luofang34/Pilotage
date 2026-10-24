use pilotage_protocol::wire;

use super::{read_blocking, write};

fn handover(state: wire::MissionHandoverState) -> wire::MissionHandover {
    wire::MissionHandover {
        assignment_id: "TEAM/run-1/handover-1".to_owned(),
        generation: 2,
        from_member: "SIM-1".to_owned(),
        to_member: "SIM-2".to_owned(),
        state: state as i32,
        task_ids: vec!["p#3".to_owned(), "p#4".to_owned()],
        basis: "SIM-1 released the work".to_owned(),
    }
}

#[test]
fn a_written_journal_reads_back_and_a_missing_one_is_empty() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("members-TEAM.json.handovers");
    assert!(read_blocking(&path).unwrap().is_empty());
    let written = vec![handover(wire::MissionHandoverState::Committed)];
    tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap()
        .block_on(write(&path, &written))
        .unwrap();
    assert_eq!(read_blocking(&path).unwrap(), written);
    assert!(!dir.path().join("members-TEAM.json.handovers.tmp").exists());
}

#[test]
fn a_journal_that_does_not_decode_stops_the_coordinator() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("journal");
    std::fs::write(&path, [0xff, 0xff, 0xff]).unwrap();
    assert!(read_blocking(&path).is_err());
}
