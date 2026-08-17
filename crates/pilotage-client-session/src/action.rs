//! Actions the engine returns for the transport port to execute, and the
//! typed module events it hands up to the shells.

use pilotage_protocol::wire;

use crate::catalog::Admission;

/// One unit of work the port performs on the engine's behalf, in emission
/// order within a call.
#[derive(Debug, Clone, PartialEq)]
pub enum ClientAction {
    /// Write these bytes to the bootstrap bidirectional stream.
    SendBootstrap(Vec<u8>),
    /// Send these bytes as one datagram.
    SendDatagram(Vec<u8>),
    /// Hand this typed event to the installed modules. The port routes it;
    /// it does not interpret it.
    Emit(ModuleEvent),
    /// Attempt a fresh connection at `at_ms` on the engine's monotonic
    /// clock. Supersedes any earlier scheduled attempt.
    ScheduleReconnect {
        /// Absolute monotonic instant of the next attempt.
        at_ms: u64,
    },
    /// Stop permanently. The fault is typed: retrying cannot help, and a
    /// port that reconnects anyway is overriding a decision, not helping.
    Stop(ClientFault),
}

/// A typed input for a client module, produced only by the engine.
#[derive(Debug, Clone, PartialEq)]
pub enum ModuleEvent {
    /// The host admitted this session; the catalog describes what it offers.
    Admitted(Admission),
    /// One telemetry sample from the datagram channel.
    Telemetry(Box<wire::TelemetrySample>),
    /// One authority transition from the session-events stream.
    Authority(wire::AuthorityEvent),
    /// The host answered a lease request.
    Lease(wire::LeaseResponse),
    /// The host confirmed a lease release.
    LeaseReleased(wire::LeaseReleased),
    /// The host rejected a control frame. Fencing feedback, not noise.
    ControlRejected(wire::FrameRejected),
    /// The host answered a discrete control action.
    ActionResult(wire::ControlActionResult),
    /// The host echoed a ping.
    Pong(wire::Pong),
    /// One video frame body (the v2 capture layout) from a per-source
    /// media stream. Decode and display are the platform port's.
    VideoFrame(Vec<u8>),
    /// A video stream claimed an impossible record length and was
    /// failed closed; its picture stops until the host cycles it.
    VideoStreamCorrupt {
        /// The claimed record length that broke the bound.
        claimed_bytes: usize,
    },
    /// The transport is down. When recovery is scheduled, `retry_at_ms`
    /// carries the instant; control authority is gone either way.
    ConnectionDown {
        /// Next scheduled attempt, absent when retry has stopped.
        retry_at_ms: Option<u64>,
    },
}

/// Why the engine stopped for good.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ClientFault {
    /// The host speaks a schema this client does not.
    #[error("host schema version {host} is not the supported {supported}")]
    SchemaMismatch {
        /// Version the host sent.
        host: u32,
        /// Version this client supports.
        supported: u32,
    },
    /// The bootstrap stream carried bytes that do not decode as an
    /// envelope. The session is unusable: framing loss on a reliable
    /// stream cannot self-heal.
    #[error("bootstrap stream framing failed: {detail}")]
    BootstrapFraming {
        /// Decoder detail for diagnostics.
        detail: String,
    },
}
