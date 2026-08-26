use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

const FIXTURE_ENV: &str = "PILOTAGE_STUBBORN_DESCENDANT_FIXTURE";
const EVENT_FIFO_ENV: &str = "PILOTAGE_STUBBORN_DESCENDANT_EVENT_FIFO";
const CONTROL_FIFO_ENV: &str = "PILOTAGE_STUBBORN_DESCENDANT_CONTROL_FIFO";

pub(crate) struct DescendantControl {
    path: PathBuf,
    writer: Option<std::fs::File>,
}

impl DescendantControl {
    pub(crate) fn new(path: &Path) -> Self {
        super::create_fifo(path);
        Self {
            path: path.to_owned(),
            writer: None,
        }
    }

    pub(crate) fn connect(&mut self) {
        self.writer = Some(super::open_fifo_writer(&self.path));
    }

    pub(crate) fn release(&mut self) {
        self.writer.take();
    }
}

pub(super) fn spawn_stubborn_descendant(event: &Path, control: &Path) -> Child {
    let executable = std::env::current_exe().expect("staged test executable");
    Command::new("/bin/sh")
        .args([
            "-c",
            "trap '' TERM; program=$1; shift; exec \"$program\" \"$@\"",
            "aviate-descendant",
        ])
        .arg(&executable)
        .args(["--exact", "stubborn_descendant_fixture", "--nocapture"])
        .env(FIXTURE_ENV, "1")
        .env(EVENT_FIFO_ENV, event)
        .env(CONTROL_FIFO_ENV, control)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn stubborn descendant")
}

pub(crate) fn run_stubborn_descendant_fixture() {
    if std::env::var_os(FIXTURE_ENV).is_none() {
        return;
    }
    let event = super::environment_path(EVENT_FIFO_ENV);
    let control = super::environment_path(CONTROL_FIFO_ENV);
    let _lifetime = super::open_fifo_writer(&event);
    let mut input = std::fs::File::open(control).expect("open descendant control FIFO");
    let mut sink = Vec::new();
    input
        .read_to_end(&mut sink)
        .expect("wait for descendant control EOF");
}
