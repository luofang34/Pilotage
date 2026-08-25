use std::io::{Read as _, Write as _};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc;
use std::thread::JoinHandle;

use flight_tune_aviate::{
    ManagedAviateProcess, PreparedAviateProcess, RecoveryOutcome, SupervisionAttestation,
};

use super::{RequestParts, TestLaunch};

const FIXTURE_ENV: &str = "PILOTAGE_DRIVER_FIXTURE";
const MODE_ENV: &str = "PILOTAGE_DRIVER_MODE";
const READY_FIFO_ENV: &str = "PILOTAGE_DRIVER_READY_FIFO";
const DESCENDANT_EVENT_ENV: &str = "PILOTAGE_DRIVER_DESCENDANT_EVENT_FIFO";
const DESCENDANT_CONTROL_ENV: &str = "PILOTAGE_DRIVER_DESCENDANT_CONTROL_FIFO";

#[derive(Clone, Copy)]
enum DriverMode {
    Prepared,
    ReleasedDescendant,
}

impl DriverMode {
    const fn name(self) -> &'static str {
        match self {
            Self::Prepared => "prepared",
            Self::ReleasedDescendant => "released_descendant",
        }
    }
}

pub(crate) struct DriverProcess {
    child: Child,
    input: Option<ChildStdin>,
    attestation: mpsc::Receiver<Vec<u8>>,
    ready_reader: Option<JoinHandle<()>>,
}

impl DriverProcess {
    pub(crate) fn spawn(launch: &TestLaunch) -> Self {
        Self::spawn_mode(launch, DriverMode::Prepared)
    }

    pub(crate) fn spawn_released_descendant(launch: &TestLaunch) -> Self {
        Self::spawn_mode(launch, DriverMode::ReleasedDescendant)
    }

    fn spawn_mode(launch: &TestLaunch, mode: DriverMode) -> Self {
        let ready_fifo = launch.driver_ready_fifo();
        let (attestation, ready_reader) = super::read_fifo_payload(&ready_fifo);
        let mut command = driver_command(launch, &ready_fifo, mode);
        let mut child = command.spawn().expect("spawn supervision driver");
        let input = child.stdin.take().expect("driver input pipe");
        Self {
            child,
            input: Some(input),
            attestation,
            ready_reader: Some(ready_reader),
        }
    }

    pub(crate) fn read_attestation(&mut self) -> SupervisionAttestation {
        let bytes = self
            .attestation
            .recv_timeout(super::EVENT_TIMEOUT)
            .expect("driver returns supervision evidence");
        self.ready_reader
            .take()
            .expect("driver evidence reader is present")
            .join()
            .expect("driver evidence reader completes");
        serde_json::from_slice(&bytes).expect("decode driver supervision evidence")
    }

    pub(crate) fn kill_and_wait(&mut self) {
        self.child.kill().expect("kill supervision driver");
        self.input.take();
        let status = self.child.wait().expect("reap supervision driver");
        assert!(!status.success(), "the driver receives a forced stop");
    }
}

pub(crate) fn run_driver_fixture() {
    if std::env::var_os(FIXTURE_ENV).is_none() {
        return;
    }
    let parts = RequestParts::from_environment();
    match std::env::var(MODE_ENV).expect("driver mode").as_str() {
        "prepared" => run_prepared(parts),
        "released_descendant" => run_released_descendant(parts),
        mode => panic!("unknown driver mode: {mode}"),
    }
}

fn run_prepared(parts: RequestParts) {
    let request = super::make_request(&parts, None);
    let prepared =
        PreparedAviateProcess::prepare_blocking(request).expect("driver prepares launch");
    publish_attestation(prepared.supervision_attestation());
    wait_for_input_eof();
    let outcome = prepared.cancel_blocking().expect("cancel driver launch");
    assert!(matches!(outcome, RecoveryOutcome::Terminal { .. }));
}

fn run_released_descendant(parts: RequestParts) {
    let event = super::environment_path(DESCENDANT_EVENT_ENV);
    let control = super::environment_path(DESCENDANT_CONTROL_ENV);
    let request = super::make_request(&parts, Some((&event, &control)));
    let prepared =
        PreparedAviateProcess::prepare_blocking(request).expect("driver prepares launch");
    let mut managed = prepared.release_blocking().expect("driver releases launch");
    publish_attestation(managed.supervision_attestation());
    wait_for_input_eof();
    terminate_managed(&mut managed);
}

fn terminate_managed(managed: &mut ManagedAviateProcess) {
    let outcome = managed
        .terminate_blocking()
        .expect("clean released driver launch");
    assert!(matches!(outcome, RecoveryOutcome::Terminal { .. }));
}

fn publish_attestation(attestation: &SupervisionAttestation) {
    let encoded = serde_json::to_vec(attestation).expect("encode driver evidence");
    let ready_fifo = super::environment_path(READY_FIFO_ENV);
    let mut ready = super::open_fifo_writer(&ready_fifo);
    ready.write_all(&encoded).expect("write driver evidence");
}

fn wait_for_input_eof() {
    let mut sink = Vec::new();
    std::io::stdin()
        .read_to_end(&mut sink)
        .expect("wait for driver input closure");
}

fn driver_command(launch: &TestLaunch, ready_fifo: &Path, mode: DriverMode) -> Command {
    let mut command = Command::new("/bin/sh");
    command
        .arg("-c")
        .arg("exec 9>\"$1\"; shift; exec \"$@\"")
        .arg("aviate-driver")
        .arg(&launch.lifecycle_fifo)
        .arg(&launch.target)
        .args(["--exact", "supervision_driver_fixture", "--nocapture"])
        .env(FIXTURE_ENV, "1")
        .env(MODE_ENV, mode.name())
        .env(READY_FIFO_ENV, ready_fifo)
        .env("PILOTAGE_DRIVER_HELPER", &launch.helper)
        .env("PILOTAGE_DRIVER_TARGET", &launch.target)
        .env("PILOTAGE_DRIVER_STORAGE_ROOT", &launch.storage_root)
        .env("PILOTAGE_DRIVER_RUNTIME_ROOT", &launch.runtime_root)
        .env("PILOTAGE_DRIVER_ARTIFACT_ROOT", &launch.artifact_root)
        .env(
            "PILOTAGE_DRIVER_TARGET_CWD",
            &launch.target_current_directory,
        )
        .env("PILOTAGE_DRIVER_TARGET_FIFO", &launch.target_fifo)
        .env(DESCENDANT_EVENT_ENV, &launch.descendant_fifo)
        .env(DESCENDANT_CONTROL_ENV, &launch.descendant_control_fifo)
        .env("PILOTAGE_DRIVER_NAME", &launch.name)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    command
}
