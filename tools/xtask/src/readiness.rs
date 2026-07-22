//! Event-based stage readiness: each stage declares the observable
//! signal that proves it is up, and the session waits on that signal
//! with a hard deadline — never on a fixed sleep.

use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

use crate::error::XtaskError;
use crate::process::ManagedChild;

/// How often probes re-check while waiting.
const POLL_INTERVAL: Duration = Duration::from_millis(300);

/// The observable signal that proves a stage is ready.
#[derive(Debug)]
pub enum Readiness {
    /// A probe command's stdout contains `needle`.
    CommandOutput {
        /// Probe program.
        program: String,
        /// Probe arguments.
        args: Vec<String>,
        /// Environment the probe needs.
        env: Vec<(String, String)>,
        /// Substring that proves readiness.
        needle: &'static str,
        /// Deadline in seconds.
        timeout_s: u64,
    },
    /// The stage's own log contains `needle`.
    LogContains {
        /// Substring that proves readiness.
        needle: &'static str,
        /// Deadline in seconds.
        timeout_s: u64,
    },
    /// A local TCP endpoint accepts connections.
    TcpAccepts {
        /// Port on 127.0.0.1.
        port: u16,
        /// Deadline in seconds.
        timeout_s: u64,
    },
    /// The host's `LISTENING <port> <cert-hex>` line appears in its log;
    /// readiness yields the certificate hash.
    HostListening {
        /// Deadline in seconds.
        timeout_s: u64,
    },
}

/// What a satisfied readiness proves, beyond "up".
#[derive(Debug, PartialEq, Eq)]
pub enum ReadySignal {
    /// The stage is up.
    Up,
    /// The host proved it is listening: the port from its LISTENING
    /// line (the one it actually bound, carried so the caller can fail
    /// closed on a mismatch instead of advertising a dead URL) and its
    /// certificate hash.
    HostListening {
        /// The port the host proved.
        port: u16,
        /// The certificate hash the host printed.
        certificate: String,
    },
}

/// Waits until `readiness` holds for `child`, failing fast if the stage
/// dies first and failing closed at the deadline.
///
/// # Errors
///
/// Returns [`XtaskError::StageDied`] when the process exits while
/// waiting, or [`XtaskError::NotReady`] at the deadline.
pub async fn await_ready(
    child: &mut ManagedChild,
    readiness: &Readiness,
) -> Result<ReadySignal, XtaskError> {
    let (timeout_s, detail) = describe(readiness);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(timeout_s);
    loop {
        child.check_running()?;
        if let Some(signal) = probe(child, readiness) {
            return Ok(signal);
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(XtaskError::NotReady {
                name: child.name,
                seconds: timeout_s,
                detail,
                log: child.log_path.clone(),
            });
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

fn describe(readiness: &Readiness) -> (u64, String) {
    match readiness {
        Readiness::CommandOutput {
            program,
            needle,
            timeout_s,
            ..
        } => (*timeout_s, format!("waiting for {needle:?} from {program}")),
        Readiness::LogContains { needle, timeout_s } => {
            (*timeout_s, format!("waiting for {needle:?} in its log"))
        }
        Readiness::TcpAccepts { port, timeout_s } => (
            *timeout_s,
            format!("waiting for 127.0.0.1:{port} to accept"),
        ),
        Readiness::HostListening { timeout_s } => {
            (*timeout_s, "waiting for the LISTENING line".to_owned())
        }
    }
}

fn probe(child: &ManagedChild, readiness: &Readiness) -> Option<ReadySignal> {
    match readiness {
        Readiness::CommandOutput {
            program,
            args,
            env,
            needle,
            ..
        } => {
            let mut command = Command::new(program);
            command.args(args);
            for (key, value) in env {
                command.env(key, value);
            }
            let output = command.output().ok()?;
            let text = String::from_utf8_lossy(&output.stdout);
            text.contains(needle).then_some(ReadySignal::Up)
        }
        Readiness::LogContains { needle, .. } => {
            let content = std::fs::read_to_string(&child.log_path).ok()?;
            content.contains(needle).then_some(ReadySignal::Up)
        }
        Readiness::TcpAccepts { port, .. } => std::net::TcpStream::connect_timeout(
            &std::net::SocketAddr::from(([127, 0, 0, 1], *port)),
            Duration::from_millis(200),
        )
        .is_ok()
        .then_some(ReadySignal::Up),
        Readiness::HostListening { .. } => {
            let content = std::fs::read_to_string(&child.log_path).ok()?;
            content
                .lines()
                .find_map(parse_listening)
                .map(|(port, certificate)| ReadySignal::HostListening { port, certificate })
        }
    }
}

/// Parses the host's `LISTENING <port> <cert-hex>` line.
pub fn parse_listening(line: &str) -> Option<(u16, String)> {
    let mut parts = line.split_whitespace();
    if parts.next()? != "LISTENING" {
        return None;
    }
    let port: u16 = parts.next()?.parse().ok()?;
    let cert = parts.next()?;
    let hex_ok = cert.len() == 64 && cert.bytes().all(|b| b.is_ascii_hexdigit());
    if !hex_ok || parts.next().is_some() {
        return None;
    }
    Some((port, cert.to_owned()))
}

/// The pinned viewer URL for a ready session. `autoconnect=1` tells the
/// viewer to connect on load — the URL already pins host, port, and
/// certificate, so a Connect click would add nothing.
pub fn viewer_url(viewer_port: u16, host_port: u16, cert: &str) -> String {
    format!(
        "http://127.0.0.1:{viewer_port}/index.html?host=127.0.0.1&port={host_port}&cert={cert}&autoconnect=1"
    )
}

/// The session manifest the static viewer serves at `/session.json`: the
/// CURRENT session's connect parameters. A viewer tab whose URL pins an
/// older session's certificate re-reads this after a failed connect and
/// converges on the live session instead of retrying a dead hash forever.
pub fn session_manifest(host_port: u16, cert: &str) -> String {
    format!("{{\"host\":\"127.0.0.1\",\"port\":{host_port},\"certHash\":\"{cert}\"}}\n")
}

/// The log file a stage writes under `log_dir`.
pub fn stage_log(log_dir: &std::path::Path, name: &str) -> PathBuf {
    log_dir.join(format!("{name}.log"))
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic)]
mod tests {
    use super::{parse_listening, viewer_url};

    #[test]
    fn listening_line_parses_exactly_and_rejects_lookalikes() {
        let cert = "a".repeat(64);
        let line = format!("LISTENING 4433 {cert}");
        assert_eq!(parse_listening(&line), Some((4433, cert.clone())));

        assert_eq!(parse_listening("LISTENING 4433"), None);
        assert_eq!(parse_listening(&format!("listening 4433 {cert}")), None);
        assert_eq!(parse_listening("LISTENING 4433 nothex"), None);
        assert_eq!(
            parse_listening(&format!("LISTENING 70000 {cert}")),
            None,
            "port must fit u16"
        );
        assert_eq!(
            parse_listening(&format!("LISTENING 4433 {cert} extra")),
            None,
            "trailing tokens are not the host's line"
        );
        let short = "ab".repeat(16);
        assert_eq!(parse_listening(&format!("LISTENING 4433 {short}")), None);
    }

    #[test]
    fn viewer_url_pins_host_port_certificate_and_autoconnect() {
        let cert = "0f".repeat(32);
        assert_eq!(
            viewer_url(8080, 4433, &cert),
            format!(
                "http://127.0.0.1:8080/index.html?host=127.0.0.1&port=4433&cert={cert}&autoconnect=1"
            )
        );
    }

    #[test]
    fn session_manifest_carries_exactly_the_listening_values() {
        let cert = "0f".repeat(32);
        // The viewer's validator requires this exact shape (host string,
        // integer port, 64-hex certHash); a drift here strands stale tabs.
        assert_eq!(
            super::session_manifest(4433, &cert),
            format!("{{\"host\":\"127.0.0.1\",\"port\":4433,\"certHash\":\"{cert}\"}}\n")
        );
    }
}
