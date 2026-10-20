use std::time::Duration;

use pilotage_adapter_api::SourceIncarnation;

/// The current step of a single enable-and-disable diagnostic.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DjiVirtualStickDiagnosticPhase {
    /// Send one enable request and await independent feedback.
    Enable,
    /// Send one disable request and await independent feedback.
    Disable,
    /// The bounded diagnostic has ended. Inspect both evidence records.
    Finished,
}

/// Evidence for one mode request. All times use the adapter's monotonic origin.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DjiVirtualStickModeEvidence {
    /// The request sequence reserved on this attachment.
    pub sequence: u16,
    /// At least one byte may have reached the transport.
    pub write_started: bool,
    /// A complete request was written and recorded by this time.
    pub written_at: Option<Duration>,
    /// The first matching result code. Zero is acceptance, not ownership.
    pub acknowledgement: Option<u8>,
    /// Receive time of the first matching acknowledgement.
    pub acknowledged_at: Option<Duration>,
    /// A new, forward report matched the requested enable flag at this time.
    pub observed_at: Option<Duration>,
    /// The independent, forward source sequence that matched the mode.
    pub observed_sequence: Option<u16>,
}

/// A reason the diagnostic did not establish the requested exchange.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DjiVirtualStickDiagnosticReason {
    /// The caller cancelled the diagnostic.
    Cancelled,
    /// The enable request did not reach a confirmed state before its deadline.
    EnableUnconfirmed,
    /// The disable request did not reach a confirmed state before its deadline.
    DisableUnconfirmed,
    /// The source identity or freshness binding was lost.
    SourceChanged,
    /// A transport or recording failure made the write outcome uncertain.
    TransportUncertain,
}

/// The retained result of an explicit mode diagnostic.
///
/// This is not flight authority, physical-stop evidence, or an input watchdog.
/// The caller must keep stepping the adapter until the diagnostic ends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DjiVirtualStickDiagnosticStatus {
    /// The caller's diagnostic identity.
    pub request_id: u64,
    /// The attachment on which the requests were made.
    pub incarnation: SourceIncarnation,
    /// The observed Mini 3 model binding.
    pub model_revision: u64,
    /// The current bounded exchange step.
    pub phase: DjiVirtualStickDiagnosticPhase,
    /// Independent enable write, acknowledgement, and observation evidence.
    pub enable: DjiVirtualStickModeEvidence,
    /// Independent disable write, acknowledgement, and observation evidence.
    pub disable: DjiVirtualStickModeEvidence,
    /// Original absolute enable-write deadline relative to the adapter origin.
    pub enable_send_until: Duration,
    /// Fixed enable-feedback deadline relative to the adapter origin.
    pub enable_observe_until: Duration,
    /// Fixed disable-write deadline relative to the adapter origin.
    pub disable_send_until: Duration,
    /// Fixed final feedback deadline relative to the adapter origin.
    pub complete_until: Duration,
    /// A diagnostic failure or cancellation. Successful cleanup does not erase it.
    pub reason: Option<DjiVirtualStickDiagnosticReason>,
}

/// A mode diagnostic could not be admitted. No command is implied by this error.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum DjiVirtualStickDiagnosticError {
    /// The separate diagnostic opt-in is disabled or the transport is unsuitable.
    #[error("virtual-stick diagnostics require explicit upper-USB observed-mode opt-in")]
    Disabled,
    /// Only one diagnostic is allowed per attachment.
    #[error("a virtual-stick diagnostic was already attempted on this attachment")]
    AlreadyAttempted,
    /// The absolute deadline is expired, too long, or the request ID is zero.
    #[error("diagnostic requires a nonzero identity and an original deadline within 250 ms")]
    InvalidDeadline,
    /// Required independent model, stopped-motor, battery, or mode data is missing.
    #[error("fresh Mini 3, stopped-motor, battery and virtual-stick data are required")]
    MissingFeedback,
    /// Existing aircraft settings do not permit this diagnostic.
    #[error(
        "diagnostic requires virtual-stick disabled, multiple modes enabled and novice mode disabled"
    )]
    Configuration,
    /// A command or incomplete receive batch prevents a mode request.
    #[error("the DJI command writer is not idle and ready")]
    Busy,
}
