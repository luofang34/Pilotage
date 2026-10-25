use pilotage_protocol::wire;

use super::{LaneJournal, read};

fn record(run_id: &str) -> wire::MissionLaneRecord {
    wire::MissionLaneRecord {
        proposal: Some(wire::MissionProposalRecord::default()),
        run_id: run_id.to_owned(),
        state: wire::MissionLaneState::Running as i32,
        ..wire::MissionLaneRecord::default()
    }
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a runtime")
}

#[test]
fn a_journal_holds_the_last_run_kept_and_no_run_after_it_ends() {
    let directory = tempfile::tempdir().expect("a directory");
    let path = directory.path().join("lane.journal");
    runtime().block_on(async {
        assert_eq!(read(&path).await.expect("no journal yet"), None);
        let mut journal = LaneJournal::new(Some(path.clone()), None);
        assert!(journal.keep(Some(record("run-1"))).await);
        assert_eq!(read(&path).await.expect("read"), Some(record("run-1")));
        assert!(journal.keep(None).await);
        assert_eq!(read(&path).await.expect("read"), None);
    });
}

#[test]
fn a_journal_that_cannot_be_written_says_so_until_it_can() {
    let directory = tempfile::tempdir().expect("a directory");
    let gone = directory.path().join("gone");
    let path = gone.join("lane.journal");
    runtime().block_on(async {
        let mut journal = LaneJournal::new(Some(path.clone()), None);
        let mut ended = record("run-1");
        ended.tasks.push(wire::MissionTaskRecord {
            task_id: "read".to_owned(),
            execution: wire::MissionTaskExecution::Completed as i32,
            results: Vec::new(),
        });
        let recorded = journal.ended();
        assert!(!journal.keep(Some(ended.clone())).await);
        assert_eq!(*recorded.borrow(), 0, "the end is not held");
        std::fs::create_dir(&gone).expect("the directory again");
        assert!(journal.keep(Some(ended.clone())).await);
        assert_eq!(*recorded.borrow(), 1);
        assert_eq!(read(&path).await.expect("read"), Some(ended));
    });
}

#[test]
fn a_journal_that_does_not_decode_stops_the_host() {
    let directory = tempfile::tempdir().expect("a directory");
    let path = directory.path().join("lane.journal");
    std::fs::write(&path, [0xff, 0xff, 0xff]).expect("written");
    let refused = runtime().block_on(read(&path));
    assert!(refused.is_err(), "{refused:?}");
}

#[test]
fn a_host_without_a_journal_starts_its_work() {
    runtime().block_on(async {
        let mut journal = LaneJournal::new(None, None);
        assert!(journal.keep(Some(record("run-1"))).await);
    });
}
