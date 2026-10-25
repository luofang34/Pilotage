use std::time::Duration;

use pilotage_mission::{OperationAnswer, PlannedOperation};
use pilotage_protocol::wire;
use tokio::sync::{mpsc, watch};

use super::executors::{read_value_blocking, write_report_blocking};
use super::{OperationHost, RunInputs, registry, spawn_operation_run};
use crate::runtime::automation::{AutomationStatus, RunControl};

/// A run with no earlier results, whose journal holds every task's end.
fn inputs() -> RunInputs {
    RunInputs {
        results: Vec::new(),
        recorded: watch::channel(usize::MAX).1,
    }
}

fn given(name: &str, value: &str) -> wire::MissionParameter {
    wire::MissionParameter {
        name: name.to_owned(),
        source: Some(wire::mission_parameter::Source::Value(value.to_owned())),
    }
}

fn result_of(name: &str, task: &str) -> wire::MissionParameter {
    wire::MissionParameter {
        name: name.to_owned(),
        source: Some(wire::mission_parameter::Source::ResultOf(
            wire::MissionResultRef {
                task_id: task.to_owned(),
                result: "value".to_owned(),
            },
        )),
    }
}

fn step(
    task: &str,
    executor: &str,
    operation: &str,
    parameters: Vec<wire::MissionParameter>,
) -> PlannedOperation {
    PlannedOperation {
        task_id: task.to_owned(),
        operation: wire::MissionOperation {
            executor: executor.to_owned(),
            operation: operation.to_owned(),
            version: 1,
            parameters,
            met_when: None,
        },
    }
}

/// `planned`, met only when the person confirms.
fn met_when_confirmed(mut planned: PlannedOperation) -> PlannedOperation {
    planned.operation.met_when = Some(wire::MissionValue {
        name: "confirmed".to_owned(),
        value: "true".to_owned(),
    });
    planned
}

/// Read the `temperature` source, ask whether it is right, write a report.
fn plan() -> Vec<PlannedOperation> {
    vec![
        step(
            "read",
            "host",
            "read-value",
            vec![given("source", "temperature")],
        ),
        met_when_confirmed(step(
            "confirm",
            "person",
            "confirm",
            vec![
                given("question", "Is this right?"),
                result_of("subject", "read"),
            ],
        )),
        step(
            "write",
            "host",
            "write-report",
            vec![
                given("title", "Cabin temperature"),
                result_of("value", "read"),
            ],
        ),
    ]
}

struct Host {
    _dir: tempfile::TempDir,
    host: OperationHost,
}

fn host() -> Host {
    let dir = tempfile::tempdir().unwrap();
    let sources = dir.path().join("sources");
    let reports = dir.path().join("reports");
    std::fs::create_dir_all(&sources).unwrap();
    std::fs::create_dir_all(&reports).unwrap();
    std::fs::write(sources.join("temperature"), "21.5 C\n").unwrap();
    Host {
        _dir: dir,
        host: OperationHost {
            sources: Some(sources),
            reports: Some(reports),
        },
    }
}

async fn until(
    status: &mut watch::Receiver<AutomationStatus>,
    what: impl Fn(&AutomationStatus) -> bool,
) -> AutomationStatus {
    tokio::time::timeout(Duration::from_secs(10), status.wait_for(|s| what(s)))
        .await
        .expect("in time")
        .expect("the run reports")
        .clone()
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .unwrap()
}

fn answer(operation_id: &str, accepted: bool) -> RunControl {
    RunControl::Answer(OperationAnswer {
        operation_id: operation_id.to_owned(),
        accepted,
        answered_by: "operator".to_owned(),
    })
}

#[test]
fn a_confirmed_value_is_written_in_a_report() {
    runtime().block_on(async {
        let host = host();
        let (controls, receiver) = mpsc::channel(4);
        let (handle, mut status) = spawn_operation_run(
            "run-1".to_owned(),
            plan(),
            host.host.clone(),
            receiver,
            inputs(),
        );
        let asked = until(&mut status, |s| !s.questions.is_empty()).await;
        let (task, question) = &asked.questions[0];
        assert_eq!(task, "confirm");
        assert_eq!(question.subject, "21.5 C", "the person sees the value read");
        assert_eq!(question.operation_id, "run-1/confirm/1");
        controls
            .send(answer("run-1/confirm/1", true))
            .await
            .unwrap();
        let done = until(&mut status, |s| s.plan_complete).await;
        assert_eq!(done.finished_tasks, ["read", "confirm", "write"]);
        assert!(done.questions.is_empty());
        let report =
            std::fs::read_to_string(host.host.reports.unwrap().join("run-1_2fwrite_2f1.txt"))
                .unwrap();
        assert!(
            report.contains("Cabin temperature") && report.contains("21.5 C"),
            "{report}"
        );
        handle.await.unwrap();
    });
}

#[test]
fn a_declined_value_ends_the_run_and_writes_nothing() {
    runtime().block_on(async {
        let host = host();
        let (controls, receiver) = mpsc::channel(4);
        let (handle, mut status) = spawn_operation_run(
            "run-2".to_owned(),
            plan(),
            host.host.clone(),
            receiver,
            inputs(),
        );
        until(&mut status, |s| !s.questions.is_empty()).await;
        controls
            .send(answer("run-2/confirm/1", false))
            .await
            .unwrap();
        let ended = until(&mut status, |s| s.ended_because.is_some()).await;
        assert_eq!(
            ended.ended_because,
            Some("a step's result does not meet its plan")
        );
        assert_eq!(ended.finished_tasks, ["read", "confirm"]);
        assert!(!ended.plan_complete);
        let written = std::fs::read_dir(host.host.reports.unwrap())
            .unwrap()
            .count();
        assert_eq!(written, 0, "no report");
        handle.await.unwrap();
    });
}

#[test]
fn an_abort_while_a_person_is_asked_stops_at_once() {
    runtime().block_on(async {
        let host = host();
        let (controls, receiver) = mpsc::channel(4);
        let (handle, mut status) =
            spawn_operation_run("run-3".to_owned(), plan(), host.host, receiver, inputs());
        until(&mut status, |s| !s.questions.is_empty()).await;
        // An answer for another operation changes nothing.
        controls.send(answer("run-3/other/1", true)).await.unwrap();
        controls.send(RunControl::Abort).await.unwrap();
        let stopped = until(&mut status, |s| s.stopped).await;
        assert_eq!(
            stopped.finished_tasks,
            ["read"],
            "the question was not answered"
        );
        assert_eq!(
            stopped.cancelled_tasks,
            ["confirm"],
            "the lane hears how it ended"
        );
        assert!(stopped.questions.is_empty());
        handle.await.unwrap();
    });
}

#[test]
fn a_report_is_written_once_for_its_operation() {
    let host = host();
    let reports = host.host.reports.unwrap();
    let first = write_report_blocking(&reports, "run-9/write/1", "Title", "one").unwrap();
    let again = write_report_blocking(&reports, "run-9/write/1", "Title", "two").unwrap();
    assert_eq!(first, again);
    let text = std::fs::read_to_string(reports.join(first)).unwrap();
    assert!(
        text.contains("one") && !text.contains("two"),
        "a repeat finds it: {text}"
    );
}

#[test]
fn a_source_is_read_only_by_its_plain_name() {
    let host = host();
    let sources = host.host.sources.unwrap();
    assert_eq!(
        read_value_blocking(&sources, "temperature").unwrap(),
        "21.5 C"
    );
    for name in ["../temperature", ".hidden", "a/b", ""] {
        assert!(read_value_blocking(&sources, name).is_err(), "{name:?}");
    }
}

#[test]
fn a_host_registers_only_what_it_is_configured_for() {
    let names = |host: &OperationHost| -> Vec<&str> {
        registry(host)
            .operations()
            .iter()
            .map(|d| d.operation)
            .collect()
    };
    assert_eq!(names(&OperationHost::default()), ["confirm"]);
    assert_eq!(
        names(&host().host),
        ["read-value", "confirm", "write-report"]
    );
}

#[test]
fn an_operation_in_another_version_fails_and_is_reported() {
    runtime().block_on(async {
        let host = host();
        let (_controls, receiver) = mpsc::channel(4);
        let mut read = step(
            "read",
            "host",
            "read-value",
            vec![given("source", "temperature")],
        );
        read.operation.version = 2;
        let (handle, mut status) = spawn_operation_run(
            "run-4".to_owned(),
            vec![read],
            host.host,
            receiver,
            inputs(),
        );
        let ended = until(&mut status, |s| s.ended_because.is_some()).await;
        assert_eq!(ended.failed_tasks, ["read"]);
        assert!(ended.finished_tasks.is_empty());
        handle.await.unwrap();
    });
}
