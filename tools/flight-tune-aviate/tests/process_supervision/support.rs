use std::collections::BTreeMap;
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc;
use std::thread::JoinHandle;
use std::time::Duration;

use flight_tune_aviate::{SupervisedProcessRequest, TargetProcessContract};

#[path = "descendant.rs"]
mod descendant;
#[path = "driver.rs"]
mod driver;
#[path = "evidence_fixtures.rs"]
mod evidence_fixtures;

pub(super) use descendant::{DescendantControl, run_stubborn_descendant_fixture};
pub(super) use driver::{DriverProcess, run_driver_fixture};
pub(super) use evidence_fixtures::{
    add_conflicting_recovery_receipt, add_linked_temporary, add_unknown_storage_object,
    add_unlinked_temporary, digest_bytes, replace_file_bytes,
};

const EVENT_TIMEOUT: Duration = Duration::from_secs(15);
const CLOSED_GATE_TIMEOUT: Duration = Duration::from_millis(250);
const TARGET_FIFO_ENV: &str = "PILOTAGE_TARGET_FIFO";
pub(super) const TARGET_ESCAPE_GROUP_ENV: &str = "PILOTAGE_TARGET_ESCAPE_GROUP";
const DESCENDANT_FIFO_ENV: &str = "PILOTAGE_DESCENDANT_FIFO";
const DESCENDANT_CONTROL_FIFO_ENV: &str = "PILOTAGE_DESCENDANT_CONTROL_FIFO";
const UNRELATED_FIFO_ENV: &str = "PILOTAGE_UNRELATED_FIFO";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FifoEvent {
    Open,
    Eof,
}

pub(super) struct FifoWatch {
    path: PathBuf,
    events: mpsc::Receiver<FifoEvent>,
    reader: Option<JoinHandle<()>>,
}

impl FifoWatch {
    pub(super) fn new(path: &Path) -> Self {
        create_fifo(path);
        let path = path.to_owned();
        let reader_path = path.clone();
        let (sender, events) = mpsc::channel();
        let reader = std::thread::Builder::new()
            .name("aviate-test-fifo-watch".to_owned())
            .spawn(move || watch_fifo(&reader_path, &sender))
            .expect("spawn FIFO watcher");
        Self {
            path,
            events,
            reader: Some(reader),
        }
    }

    pub(super) fn expect_open(&self, detail: &str) {
        self.expect_event(FifoEvent::Open, detail);
    }

    pub(super) fn expect_no_event(&self, detail: &str) {
        let result = self.events.recv_timeout(CLOSED_GATE_TIMEOUT);
        assert!(
            matches!(result, Err(mpsc::RecvTimeoutError::Timeout)),
            "{detail}"
        );
    }

    pub(super) fn expect_eof(mut self, detail: &str) {
        self.expect_event(FifoEvent::Eof, detail);
        self.join_reader();
    }

    pub(super) fn expect_unused(mut self, detail: &str) {
        assert!(
            matches!(self.events.try_recv(), Err(mpsc::TryRecvError::Empty)),
            "{detail}"
        );
        let writer = std::fs::OpenOptions::new()
            .write(true)
            .open(&self.path)
            .expect("open unused FIFO");
        self.expect_event(FifoEvent::Open, "the FIFO watcher accepts the test writer");
        drop(writer);
        self.expect_event(FifoEvent::Eof, "the unused FIFO test writer closes");
        self.join_reader();
    }

    fn expect_event(&self, expected: FifoEvent, detail: &str) {
        let event = self
            .events
            .recv_timeout(EVENT_TIMEOUT)
            .unwrap_or_else(|_| panic!("timed out: {detail}"));
        assert_eq!(event, expected, "{detail}");
    }

    fn join_reader(&mut self) {
        self.reader
            .take()
            .expect("FIFO watcher is present")
            .join()
            .expect("FIFO watcher completes");
    }
}

pub(super) struct TestLaunch {
    _root: tempfile::TempDir,
    helper: PathBuf,
    target: PathBuf,
    storage_root: PathBuf,
    runtime_root: PathBuf,
    artifact_root: PathBuf,
    target_current_directory: PathBuf,
    target_fifo: PathBuf,
    descendant_fifo: PathBuf,
    descendant_control_fifo: PathBuf,
    lifecycle_fifo: PathBuf,
    unrelated_fifo: PathBuf,
    name: String,
}

impl TestLaunch {
    pub(super) fn new(name: &str) -> Self {
        let temporary_parent = std::fs::canonicalize("/tmp").expect("canonical temporary parent");
        let root = tempfile::Builder::new()
            .prefix("aviate-supervision-")
            .tempdir_in(temporary_parent)
            .expect("create test root");
        let runtime_root = root.path().join("runtime");
        let target_current_directory = root.path().join("target-cwd");
        create_private_directory(&runtime_root);
        create_private_directory(&target_current_directory);
        let helper = std::fs::canonicalize(env!("CARGO_BIN_EXE_flight_tune_aviate_supervisor"))
            .expect("canonical helper");
        let target = std::fs::canonicalize(std::env::current_exe().expect("test executable"))
            .expect("canonical target");
        Self {
            storage_root: root.path().join("storage"),
            artifact_root: root.path().join("artifacts"),
            target_fifo: root.path().join("target.fifo"),
            descendant_fifo: root.path().join("descendant.fifo"),
            descendant_control_fifo: root.path().join("descendant-control.fifo"),
            lifecycle_fifo: root.path().join("lifecycle.fifo"),
            unrelated_fifo: root.path().join("unrelated.fifo"),
            _root: root,
            helper,
            target,
            runtime_root,
            target_current_directory,
            name: name.to_owned(),
        }
    }

    pub(super) fn request(&self) -> SupervisedProcessRequest {
        make_request(&RequestParts::from_launch(self), None)
    }

    pub(super) fn descendant_request(&self) -> SupervisedProcessRequest {
        make_request(
            &RequestParts::from_launch(self),
            Some((&self.descendant_fifo, &self.descendant_control_fifo)),
        )
    }

    pub(super) fn target_fifo(&self) -> &Path {
        &self.target_fifo
    }

    pub(super) fn descendant_fifo(&self) -> &Path {
        &self.descendant_fifo
    }

    pub(super) fn descendant_control_fifo(&self) -> &Path {
        &self.descendant_control_fifo
    }

    pub(super) fn lifecycle_fifo(&self) -> &Path {
        &self.lifecycle_fifo
    }

    pub(super) fn unrelated_fifo(&self) -> &Path {
        &self.unrelated_fifo
    }

    pub(super) fn storage_root(&self) -> &Path {
        &self.storage_root
    }

    pub(super) fn runtime_root(&self) -> &Path {
        &self.runtime_root
    }

    fn driver_ready_fifo(&self) -> PathBuf {
        self._root.path().join("driver-ready.fifo")
    }
}

struct RequestParts {
    helper: PathBuf,
    target: PathBuf,
    storage_root: PathBuf,
    runtime_root: PathBuf,
    artifact_root: PathBuf,
    target_current_directory: PathBuf,
    target_fifo: PathBuf,
    name: String,
}

impl RequestParts {
    fn from_launch(launch: &TestLaunch) -> Self {
        Self {
            helper: launch.helper.clone(),
            target: launch.target.clone(),
            storage_root: launch.storage_root.clone(),
            runtime_root: launch.runtime_root.clone(),
            artifact_root: launch.artifact_root.clone(),
            target_current_directory: launch.target_current_directory.clone(),
            target_fifo: launch.target_fifo.clone(),
            name: launch.name.clone(),
        }
    }

    fn from_environment() -> Self {
        Self {
            helper: environment_path("PILOTAGE_DRIVER_HELPER"),
            target: environment_path("PILOTAGE_DRIVER_TARGET"),
            storage_root: environment_path("PILOTAGE_DRIVER_STORAGE_ROOT"),
            runtime_root: environment_path("PILOTAGE_DRIVER_RUNTIME_ROOT"),
            artifact_root: environment_path("PILOTAGE_DRIVER_ARTIFACT_ROOT"),
            target_current_directory: environment_path("PILOTAGE_DRIVER_TARGET_CWD"),
            target_fifo: environment_path("PILOTAGE_DRIVER_TARGET_FIFO"),
            name: std::env::var("PILOTAGE_DRIVER_NAME").expect("driver run name"),
        }
    }
}

pub(super) struct UnrelatedProcess {
    child: Child,
    input: Option<ChildStdin>,
}

impl UnrelatedProcess {
    pub(super) fn spawn(fifo: &Path) -> Self {
        let mut child = Command::new(std::env::current_exe().expect("test executable"))
            .args(["--exact", "unrelated_process_fixture", "--nocapture"])
            .env(UNRELATED_FIFO_ENV, fifo)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn unrelated process");
        let input = child.stdin.take().expect("unrelated input pipe");
        Self {
            child,
            input: Some(input),
        }
    }

    pub(super) fn expect_running(&mut self) {
        assert!(
            self.child
                .try_wait()
                .expect("inspect unrelated process")
                .is_none(),
            "invalid recovery does not stop the unrelated process"
        );
    }

    pub(super) fn stop_and_wait(&mut self) {
        self.input.take();
        let status = self.child.wait().expect("reap unrelated process");
        assert!(status.success(), "the unrelated process exits normally");
    }
}

pub(super) fn run_target_fixture() {
    let Some(path) = std::env::var_os(TARGET_FIFO_ENV) else {
        return;
    };
    if std::env::var_os(TARGET_ESCAPE_GROUP_ENV).is_some() {
        let session = rustix::process::setsid().expect("target creates escaped session");
        let own_pid = i32::try_from(std::process::id()).expect("target PID fits POSIX");
        assert_eq!(session.as_raw_pid(), own_pid);
    }
    let _target_lifetime = open_fifo_writer(Path::new(&path));
    let _descendant = match (
        std::env::var_os(DESCENDANT_FIFO_ENV),
        std::env::var_os(DESCENDANT_CONTROL_FIFO_ENV),
    ) {
        (Some(event), Some(control)) => Some(descendant::spawn_stubborn_descendant(
            Path::new(&event),
            Path::new(&control),
        )),
        _ => None,
    };
    loop {
        std::thread::park();
    }
}

pub(super) fn run_unrelated_fixture() {
    let Some(path) = std::env::var_os(UNRELATED_FIFO_ENV) else {
        return;
    };
    let _lifetime = open_fifo_writer(Path::new(&path));
    let mut sink = Vec::new();
    std::io::stdin()
        .read_to_end(&mut sink)
        .expect("wait for unrelated process input closure");
}

fn make_request(
    parts: &RequestParts,
    descendant_fifos: Option<(&Path, &Path)>,
) -> SupervisedProcessRequest {
    let mut environment = BTreeMap::new();
    environment.insert(
        TARGET_FIFO_ENV.to_owned(),
        utf8_path(&parts.target_fifo).to_owned(),
    );
    if let Some((event, control)) = descendant_fifos {
        environment.insert(DESCENDANT_FIFO_ENV.to_owned(), utf8_path(event).to_owned());
        environment.insert(
            DESCENDANT_CONTROL_FIFO_ENV.to_owned(),
            utf8_path(control).to_owned(),
        );
    }
    SupervisedProcessRequest {
        supervisor_executable: parts.helper.clone(),
        supervisor_executable_digest: digest_file(&parts.helper),
        target_executable: parts.target.clone(),
        target_executable_digest: digest_file(&parts.target),
        target_arguments: vec![
            "--exact".to_owned(),
            "supervised_target_fixture".to_owned(),
            "--nocapture".to_owned(),
        ],
        target_environment: environment,
        target_process_contract: TargetProcessContract::RetainProcessGroup,
        target_current_directory: parts.target_current_directory.clone(),
        storage_root: parts.storage_root.clone(),
        runtime_root: parts.runtime_root.clone(),
        artifact_root: parts.artifact_root.clone(),
        run_intent_digest: digest_bytes(format!("integration-{}", parts.name).as_bytes()),
        startup_timeout: EVENT_TIMEOUT,
        cleanup_timeout: EVENT_TIMEOUT,
    }
}

pub(super) fn create_fifo(path: &Path) {
    let status = Command::new("mkfifo")
        .arg(path)
        .status()
        .expect("run mkfifo");
    assert!(status.success(), "mkfifo creates the event channel");
}

fn watch_fifo(path: &Path, sender: &mpsc::Sender<FifoEvent>) {
    let mut file = std::fs::File::open(path).expect("open FIFO reader");
    sender.send(FifoEvent::Open).expect("send FIFO open event");
    let mut sink = Vec::new();
    file.read_to_end(&mut sink).expect("read FIFO to EOF");
    sender.send(FifoEvent::Eof).expect("send FIFO EOF event");
}

fn read_fifo_payload(path: &Path) -> (mpsc::Receiver<Vec<u8>>, JoinHandle<()>) {
    create_fifo(path);
    let path = path.to_owned();
    let (sender, receiver) = mpsc::channel();
    let reader = std::thread::Builder::new()
        .name("aviate-test-fifo-payload".to_owned())
        .spawn(move || {
            let mut file = std::fs::File::open(path).expect("open FIFO payload reader");
            let mut payload = Vec::new();
            file.read_to_end(&mut payload).expect("read FIFO payload");
            sender.send(payload).expect("send FIFO payload");
        })
        .expect("spawn FIFO payload reader");
    (receiver, reader)
}

pub(super) fn open_fifo_writer(path: &Path) -> std::fs::File {
    std::fs::OpenOptions::new()
        .write(true)
        .open(path)
        .expect("open FIFO writer")
}

fn create_private_directory(path: &Path) {
    use std::os::unix::fs::DirBuilderExt as _;

    let mut builder = std::fs::DirBuilder::new();
    builder.mode(0o700);
    builder.create(path).expect("create private directory");
}

fn digest_file(path: &Path) -> flight_tune::Digest {
    digest_bytes(&std::fs::read(path).expect("read executable"))
}

pub(super) fn kill_process(pid: u32) {
    let raw = i32::try_from(pid).expect("test process PID fits POSIX");
    let pid = rustix::process::Pid::from_raw(raw).expect("test process PID is nonzero");
    rustix::process::kill_process(pid, rustix::process::Signal::KILL)
        .expect("signal exact test process");
}

pub(super) fn environment_path(name: &str) -> PathBuf {
    PathBuf::from(std::env::var_os(name).unwrap_or_else(|| panic!("missing {name}")))
}

fn utf8_path(path: &Path) -> &str {
    path.to_str().expect("test path is UTF-8")
}
