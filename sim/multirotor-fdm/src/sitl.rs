//! The Betaflight SITL child process: configuration load, launch, output
//! relay, and exit detection. What to do when is the supervisor's call
//! (`pilotage_multirotor_fdm::supervisor`); this module only does it.

use std::io::{BufRead, BufReader, Read};
use std::net::{TcpListener, UdpSocket};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;

use pilotage_multirotor_fdm::supervisor::Event;

/// The line Betaflight prints when its MSP port binds, late in its init.
/// It goes to stderr, which is unbuffered; Betaflight's own init lines go
/// to stdout, which a pipe block-buffers, so they arrive too late to time
/// anything by.
const INIT_MARKER: &str = "bind port 5761";
/// Ports Betaflight SITL binds; it has no option to move them.
const SITL_UDP_PORT: u16 = 9_003;
const SITL_MSP_PORT: u16 = 5_761;

/// Where Betaflight lives and what it loads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SitlConfig {
    /// `betaflight_SITL.elf`.
    pub(crate) executable: PathBuf,
    /// Its working directory, where it keeps `eeprom.bin`.
    pub(crate) work_dir: PathBuf,
    /// A CLI file to load before the first launch.
    pub(crate) config: Option<PathBuf>,
}

/// Why the SITL could not be run.
#[derive(Debug, thiserror::Error)]
pub(crate) enum SitlError {
    #[error(
        "{what} is already bound, so another Betaflight SITL is running (one fits per machine)"
    )]
    PortBusy { what: &'static str },
    #[error("starting {} failed: {source}", path.display())]
    Spawn {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// The running (or exited) child and its output relay.
pub(crate) struct Sitl {
    config: SitlConfig,
    child: Option<Child>,
    lines: mpsc::Receiver<String>,
    line_sender: mpsc::Sender<String>,
}

impl Sitl {
    /// Checks that no other SITL holds Betaflight's fixed ports. Relative
    /// paths are taken from this process's directory, not Betaflight's.
    pub(crate) fn new(mut config: SitlConfig) -> Result<Self, SitlError> {
        config.config = config
            .config
            .map(|file| std::path::absolute(&file).unwrap_or(file));
        config.executable = std::path::absolute(&config.executable).unwrap_or(config.executable);
        UdpSocket::bind(("0.0.0.0", SITL_UDP_PORT))
            .map_err(|_| SitlError::PortBusy { what: "UDP 9003" })?;
        TcpListener::bind(("0.0.0.0", SITL_MSP_PORT))
            .map_err(|_| SitlError::PortBusy { what: "TCP 5761" })?;
        let (line_sender, lines) = mpsc::channel();
        Ok(Self {
            config,
            child: None,
            lines,
            line_sender,
        })
    }

    /// Whether a configuration file is loaded before the first launch.
    pub(crate) fn provisions(&self) -> bool {
        self.config.config.is_some()
    }

    /// Runs Betaflight once with `--config` and waits for it; returns
    /// whether it loaded and saved the file.
    pub(crate) fn provision_blocking(&self) -> Result<bool, SitlError> {
        let Some(file) = &self.config.config else {
            return Ok(true);
        };
        let output = Command::new(&self.config.executable)
            .arg("--config")
            .arg(file)
            .current_dir(&self.config.work_dir)
            .stdin(Stdio::null())
            .output()
            .map_err(|source| self.spawn_error(source))?;
        let saved = output.status.success()
            && String::from_utf8_lossy(&output.stdout).contains("EEPROM saved");
        if !saved {
            let stderr = String::from_utf8_lossy(&output.stderr);
            tracing::warn!(
                status = %output.status,
                stderr = stderr.lines().last().unwrap_or_default(),
                "Betaflight did not load the configuration file"
            );
        }
        Ok(saved)
    }

    /// Starts Betaflight and relays its output.
    pub(crate) fn launch(&mut self) -> Result<(), SitlError> {
        let mut child = Command::new(&self.config.executable)
            .current_dir(&self.config.work_dir)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|source| self.spawn_error(source))?;
        if let Some(out) = child.stdout.take() {
            relay(out, self.line_sender.clone());
        }
        if let Some(err) = child.stderr.take() {
            relay(err, self.line_sender.clone());
        }
        tracing::info!(pid = child.id(), "Betaflight SITL launched");
        self.child = Some(child);
        Ok(())
    }

    /// Kills the child; its exit is reported by the next [`Sitl::poll`].
    pub(crate) fn kill(&mut self) {
        if let Some(child) = self.child.as_mut() {
            child.kill().ok();
        }
    }

    /// Relays the child's new output and reports what it shows.
    pub(crate) fn poll(&mut self) -> Vec<Event> {
        let mut events = Vec::new();
        while let Ok(line) = self.lines.try_recv() {
            // Thousands of these at every boot; they say nothing.
            if !line.starts_with("[FLASH_") {
                tracing::info!(target: "betaflight", "{line}");
            }
            if line.contains(INIT_MARKER) {
                events.push(Event::InitComplete);
            }
        }
        let exited = self
            .child
            .as_mut()
            .is_some_and(|child| !matches!(child.try_wait(), Ok(None)));
        if exited {
            if let Some(status) = self.child.take().and_then(|mut c| c.wait().ok()) {
                tracing::warn!(%status, "Betaflight SITL exited");
            }
            events.push(Event::Exited);
        }
        events
    }

    fn spawn_error(&self, source: std::io::Error) -> SitlError {
        SitlError::Spawn {
            path: self.config.executable.clone(),
            source,
        }
    }
}

impl Drop for Sitl {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            child.kill().ok();
            child.wait().ok();
        }
    }
}

/// Forwards every line of `stream` until it closes.
fn relay(stream: impl Read + Send + 'static, lines: mpsc::Sender<String>) {
    std::thread::spawn(move || {
        for line in BufReader::new(stream).lines() {
            let Ok(line) = line else { break };
            if lines.send(line).is_err() {
                break;
            }
        }
    });
}
