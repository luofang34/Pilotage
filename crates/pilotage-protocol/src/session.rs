//! Session-bootstrap domain vocabulary: handshake, scope-lease
//! request/response, RTT probing, and frame-rejection notices (ADR-0005,
//! ADR-0006, ADR-0009, ADR-0010).

use pilotage_timing::MonoTimestamp;

use crate::ids::{Generation, PrincipalId, ScopeId, SequenceNum, VehicleId};
use crate::wire;

/// The first message a client sends after the WebTransport session is
/// established (ADR-0005).
#[derive(Debug, Clone, PartialEq)]
pub struct ClientHello {
    /// Highest `pilotage.v1` schema version the client can interpret.
    pub protocol_version: u32,
    /// Human-readable client identification, for diagnostics only.
    pub client_name: String,
    /// Opaque join token proving prior admission; interpreted only by the
    /// issuing admission service, never by this crate.
    pub join_token: Vec<u8>,
}

/// A single (vehicle, scope) pair's current holder and fencing generation,
/// as reported in a `ServerWelcome` (ADR-0006, ADR-0010).
#[derive(Debug, Clone, PartialEq)]
pub struct ScopeHolderSnapshot {
    /// Vehicle the scope belongs to.
    pub vehicle: VehicleId,
    /// Control scope this snapshot describes.
    pub scope: ScopeId,
    /// Current holder, absent when the scope is unassigned.
    pub holder: Option<PrincipalId>,
    /// Fencing generation currently in force for `scope`.
    pub generation: Generation,
}

/// The host's reply to `ClientHello`, establishing session identity and
/// publishing the state the client needs to render an initial UI.
#[derive(Debug, Clone, PartialEq)]
pub struct ServerWelcome {
    /// Session identity assigned to this connection.
    pub session: crate::ids::SessionId,
    /// Principal identity assigned to this connection.
    pub principal: PrincipalId,
    /// The host's advertised capabilities (vehicles, scopes, modes).
    pub host_capabilities: wire::HostCapabilities,
    /// Current holder and generation for every (vehicle, scope) pair the
    /// host tracks.
    pub scope_holders: Vec<ScopeHolderSnapshot>,
}

/// A client's request to lease a control scope for a vehicle (ADR-0006).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeaseRequest {
    /// Vehicle the lease applies to.
    pub vehicle: VehicleId,
    /// Control scope being requested.
    pub scope: ScopeId,
}

/// A holder voluntarily relinquishing a control scope (ADR-0006): routed
/// through the authority engine's release path, so the generation advances
/// (stragglers are fenced) and the vehicle's link-loss policy engages
/// exactly as for an involuntary loss.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeaseRelease {
    /// Vehicle the released scope belongs to.
    pub vehicle: VehicleId,
    /// Control scope being relinquished.
    pub scope: ScopeId,
}

/// The host's acknowledgement of a [`LeaseRelease`]: authority is
/// relinquished the moment this arrives (the host silence watchdog remains
/// the independent backup if it never does).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeaseReleased {
    /// Vehicle the released scope belongs to.
    pub vehicle: VehicleId,
    /// The scope the release targeted.
    pub scope: ScopeId,
    /// False when the sender did not hold the scope (nothing was released).
    pub released: bool,
    /// The fencing generation now in force.
    pub generation: Generation,
}

/// The host's reliable notice that it CLEARED its link-loss latch for one scope
/// of a vehicle: the accepted neutral frame on [`Self::generation`] satisfied
/// the recovery activation condition, so the neutralize/brake policy is lifted.
/// The controlling client correlates by vehicle/scope/generation to confirm its
/// recovery landed rather than trusting a best-effort control datagram arrived
/// (ADR-0012).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkLossCleared {
    /// Vehicle whose link-loss latch cleared.
    pub vehicle: VehicleId,
    /// The scope that recovered.
    pub scope: ScopeId,
    /// The fresh generation whose accepted neutral frame cleared the latch.
    pub generation: Generation,
}

/// Why a `LeaseRequest` was denied (ADR-0006, ADR-0010).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeaseDenialReason {
    /// Another principal currently holds the scope and did not offer it.
    AlreadyHeld,
    /// The (vehicle, scope) pair is not published by the host's
    /// capabilities.
    UnknownScope,
    /// The requesting principal lacks the authority class required for
    /// this scope.
    NotAuthorized,
}

/// The host's reply to a `LeaseRequest`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeaseResponse {
    /// Vehicle the response concerns.
    pub vehicle: VehicleId,
    /// Control scope the response concerns.
    pub scope: ScopeId,
    /// Whether the lease was granted.
    pub granted: bool,
    /// New generation on grant; current (unchanged) generation on denial.
    pub generation: Generation,
    /// Meaningful only when `granted` is `false`.
    pub reason: Option<LeaseDenialReason>,
}

/// An RTT/offset probe carrying the sender's local transport-time sample.
///
/// `sender_sent_at` is a `transport_time` sample local to the sender
/// (ADR-0009) and MUST NOT be compared directly against a timestamp from a
/// different endpoint. It is only meaningful as input to
/// [`pilotage_timing`]'s RTT/offset estimator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ping {
    /// Correlates this `Ping` with its `Pong`; carries no ordering or
    /// uniqueness guarantee beyond what the sender chooses.
    pub nonce: u64,
    /// Sender-local transport-time sample at send.
    pub sender_sent_at: MonoTimestamp,
}

/// The reply to a [`Ping`], echoing its nonce and sender timestamp and
/// adding the responder's own local transport-time sample.
///
/// As with `Ping`, `responder_sent_at` is local to the responder and must
/// never be compared raw against `echoed_sender_sent_at`; both feed
/// [`pilotage_timing`]'s estimator, never a direct subtraction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pong {
    /// Echoes the originating `Ping`'s nonce.
    pub nonce: u64,
    /// Echoes the originating `Ping`'s sender-local timestamp verbatim.
    pub echoed_sender_sent_at: MonoTimestamp,
    /// Responder-local transport-time sample at reply.
    pub responder_sent_at: MonoTimestamp,
}

/// Why the host rejected an inbound control frame without applying it
/// (ADR-0009 rejection rules, CTRL-01 typed-command validation).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameRejectionReason {
    /// The frame's generation is older than the scope's current
    /// generation.
    StaleGeneration,
    /// The scope has no current holder.
    NoHolder,
    /// The (vehicle, scope) pair is not published by the host's
    /// capabilities.
    UnknownScope,
    /// The frame's sampled-at timestamp is older than the configured
    /// maximum control age.
    TooOld,
    /// The frame carried NO command representation: no legacy payload, no
    /// typed intent, and no actions.
    EmptyCommand,
    /// The frame carried BOTH the legacy payload and a typed
    /// intent/actions; exactly one representation is allowed.
    DualCommand,
    /// The typed intent's family or reference frame is not advertised for
    /// the frame's scope.
    UnsupportedIntent,
    /// A typed action (or its mode target) is not advertised for the
    /// frame's scope.
    UnsupportedAction,
    /// A typed intent component exceeds the scope's advertised magnitude
    /// limit.
    LimitExceeded,
    /// The frame's actions repeat or conflict (arm together with disarm);
    /// the frame is rejected whole, executing neither.
    ConflictingActions,
    /// A typed frame's activation revision does not match the sender's
    /// announced profile activation (or nothing was announced): the frame
    /// cannot be bound to profile evidence, so it is not applied.
    ProfileMismatch,
    /// A datagram frame carried typed discrete actions. Actions ride ONLY
    /// the reliable ordered session stream — a dropped or reordered edge
    /// changes meaning, so the droppable channel refuses them whole.
    ActionOnDatagram,
    /// A legacy payload omitted an axis its scope's translation routes;
    /// the structurally total translation would turn "no update" into an
    /// explicit neutral, so partial legacy coverage is rejected.
    PartialCommand,
}

/// Sent back to a control frame's sender (never broadcast) when the frame
/// is well-formed but not honored (ADR-0009, ADR-0012).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameRejected {
    /// Vehicle the rejected frame targeted.
    pub vehicle: VehicleId,
    /// Control scope the rejected frame targeted.
    pub scope: ScopeId,
    /// Sequence number of the rejected frame.
    pub sequence: SequenceNum,
    /// Why the frame was rejected.
    pub reason: FrameRejectionReason,
    /// The scope's fencing generation at the time of rejection.
    pub current_generation: Generation,
}

/// The explicit outcome of ONE typed discrete action, sent to the frame's
/// sender on the reliable session stream after the adapter disposed of it —
/// every accepted frame's action gets exactly one result, so an operator can
/// distinguish "arm honored" from "arm silently dropped" (CTRL-01).
#[derive(Debug, Clone, PartialEq)]
pub struct ControlActionResult {
    /// Vehicle the action targeted.
    pub vehicle: VehicleId,
    /// Control scope the action arrived under.
    pub scope: ScopeId,
    /// Fencing generation of the frame that carried the action.
    pub generation: Generation,
    /// Sequence number of the frame that carried the action, correlating
    /// the result to the exact press.
    pub sequence: SequenceNum,
    /// The action this result answers.
    pub action: crate::intent::ControlAction,
    /// Whether the adapter accepted (executed or queued) the action.
    pub accepted: bool,
    /// Adapter-supplied reason when not accepted; empty on acceptance.
    pub detail: String,
    /// Echoes the request's correlation id (zero when it carried none), so a
    /// retransmitting sender resolves exactly the press this answers.
    pub action_id: u32,
}

/// A typed discrete action on the RELIABLE ordered session stream
/// (CTRL-01): actions never ride droppable datagrams — loss, duplication,
/// or reordering changes their meaning. The full authority binding travels
/// with the request and the host validates every field against its own
/// records before the adapter sees the action.
#[derive(Debug, Clone, PartialEq)]
pub struct ControlActionCommand {
    /// The sender's session, validated against the connection's own.
    pub session: crate::ids::SessionId,
    /// The vehicle addressed.
    pub vehicle: VehicleId,
    /// The control scope the sender must hold.
    pub scope: ScopeId,
    /// The fencing generation the sender holds the scope at.
    pub generation: Generation,
    /// The sender's announced activation revision (INPUT-01 binding).
    pub activation_revision: u32,
    /// The action commanded.
    pub action: crate::intent::ControlAction,
    /// Nonzero correlation id echoed by the answering result.
    pub action_id: u32,
}

/// Announces the sender's newly activated control profile on the reliable
/// session stream, binding the activation revision its frames will carry to
/// the profile's identity, document revision, and exact content digest — the
/// traceability record for control evidence (INPUT-01).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileActivation {
    /// The session announcing the activation.
    pub session: crate::ids::SessionId,
    /// The profile's stable identity string.
    pub profile_id: String,
    /// The profile document's own revision.
    pub profile_revision: u32,
    /// The sender's monotonic activation revision; frames carry this value.
    pub activation_revision: u32,
    /// SHA-256 content digest of the exact scheme-profile bytes.
    pub digest: [u8; 32],
    /// The selected DEVICE profile completing the effective mapping; empty
    /// id (with a zero digest) when no pad profile is selected.
    pub device_profile_id: String,
    /// The device profile document's own revision.
    pub device_profile_revision: u32,
    /// SHA-256 content digest of the exact device-profile bytes.
    pub device_digest: [u8; 32],
}
