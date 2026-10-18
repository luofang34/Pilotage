//! The launch guard: what a session holds, and whether anything else
//! already holds it.
//!
//! A session is refused only for a TRUE conflict — a port it must bind
//! that is bound, or a machine-wide singleton that is running. Sessions
//! that hold different things run side by side. The guard only looks: it
//! never stops a process, because the launcher stops only what it started.

use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream, UdpSocket};
use std::process::Command;
use std::time::Duration;

use crate::error::XtaskError;

/// One thing a session holds while it runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Claim {
    /// A UDP port a session process binds.
    Udp {
        /// The port.
        port: u16,
        /// Which process binds it, for the refusal message.
        holder: String,
    },
    /// A TCP port a session process listens on.
    Tcp {
        /// The port.
        port: u16,
        /// Which process listens on it, for the refusal message.
        holder: String,
    },
    /// A machine-wide singleton: any process whose command line matches
    /// this `pgrep -f` pattern holds it.
    Process(&'static str),
    /// The default gz-transport partition. Topics such as `/camera` are
    /// not scoped by world, so two simulators in one partition mix their
    /// frames; a session without a partition of its own needs it empty.
    DefaultGzPartition,
}

/// Every claim of a session: its host and viewer ports, then the
/// backend's own.
pub(crate) fn session_claims(
    ctx: &crate::backend::SessionContext,
    backend: &dyn crate::backend::SimBackend,
) -> Vec<Claim> {
    let mut claims = vec![
        Claim::Udp {
            port: ctx.host_port,
            holder: "session host".to_owned(),
        },
        Claim::Tcp {
            port: ctx.viewer_port,
            holder: "viewer".to_owned(),
        },
    ];
    claims.extend(backend.claims(ctx));
    claims
}

/// Refuses to start when any claim is already held, naming every one.
///
/// # Errors
///
/// Returns [`XtaskError::SessionConflict`] listing each held claim.
pub(crate) fn refuse_conflicts(claims: &[Claim]) -> Result<(), XtaskError> {
    let conflicts: Vec<String> = claims.iter().filter_map(held_by_another).collect();
    if conflicts.is_empty() {
        Ok(())
    } else {
        Err(XtaskError::SessionConflict { conflicts })
    }
}

/// `Some(description)` when `claim` is held by something else now.
fn held_by_another(claim: &Claim) -> Option<String> {
    match claim {
        Claim::Udp { port, holder } => {
            udp_port_in_use(*port).then(|| format!("UDP port {port} ({holder}) is in use"))
        }
        Claim::Tcp { port, holder } => {
            tcp_port_in_use(*port).then(|| format!("TCP port {port} ({holder}) is in use"))
        }
        Claim::Process(pattern) => {
            let listing = matching_processes(pattern);
            (!listing.is_empty()).then(|| format!("{pattern:?} is running:\n{listing}"))
        }
        Claim::DefaultGzPartition => default_gz_partition_world().map(|topic| {
            format!(
                "a gz world runs in the default partition ({topic}); \
                 pick another --instance"
            )
        }),
    }
}

/// A UDP port is in use when it cannot be bound on loopback or on every
/// interface. Both, because the holders differ: the MAVLink link binds
/// loopback, PX4 and the WebTransport host bind every interface.
pub(crate) fn udp_port_in_use(port: u16) -> bool {
    [Ipv4Addr::LOCALHOST, Ipv4Addr::UNSPECIFIED]
        .into_iter()
        .any(|ip| UdpSocket::bind(SocketAddr::from((ip, port))).is_err())
}

/// A TCP port is in use when loopback accepts a connection on it or it
/// cannot be bound on loopback. The connect is needed: the standard
/// listener sets `SO_REUSEADDR`, which on BSD lets a bind succeed beside
/// a listener on every interface.
pub(crate) fn tcp_port_in_use(port: u16) -> bool {
    let loopback = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    TcpStream::connect_timeout(&loopback, Duration::from_millis(200)).is_ok()
        || TcpListener::bind(loopback).is_err()
}

/// `pid command` lines of every process matching `pattern`.
fn matching_processes(pattern: &str) -> String {
    let Ok(output) = Command::new("pgrep").args(["-f", pattern]).output() else {
        return String::new();
    };
    if !output.status.success() {
        return String::new();
    }
    let mut listing = String::new();
    for pid in String::from_utf8_lossy(&output.stdout).split_whitespace() {
        // `ps` prints the command line without the environment noise
        // macOS `pgrep -fl` appends.
        let Ok(line) = Command::new("ps")
            .args(["-o", "pid=,command=", "-p", pid])
            .output()
        else {
            continue;
        };
        listing.push_str(String::from_utf8_lossy(&line.stdout).trim_end());
        listing.push('\n');
    }
    listing.trim_end().to_owned()
}

/// The first world topic visible in the default gz partition, if any.
fn default_gz_partition_world() -> Option<String> {
    let output = Command::new("gz")
        .args(["topic", "-l"])
        .env("PATH", crate::backend::aviate_gz::search_path())
        .env("GZ_IP", "127.0.0.1")
        .env_remove("GZ_PARTITION")
        .output()
        .ok()?;
    first_world_topic(&String::from_utf8_lossy(&output.stdout))
}

/// The first `/world/...` line of a `gz topic -l` listing.
fn first_world_topic(listing: &str) -> Option<String> {
    listing
        .lines()
        .map(str::trim)
        .find(|line| line.starts_with("/world/"))
        .map(str::to_owned)
}

#[cfg(test)]
mod tests;
