//! Adapter-owned progress for a discrete flight procedure.

use pilotage_protocol::{Generation, ScopeId, SequenceNum, SessionId};
use pilotage_timing::MonoTimestamp;

use crate::{CommandOutcome, SourceIncarnation};
pub use pilotage_protocol::{FlightProcedureKind, FlightProcedurePhase, FlightProcedureReason};

/// The exact control request that started a procedure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlightProcedureRequest {
    /// Session that submitted the action.
    pub session: SessionId,
    /// Flight scope that admitted the action.
    pub scope: ScopeId,
    /// Authority generation under which the action was admitted.
    pub generation: Generation,
    /// Control frame sequence; zero for the reliable action path.
    pub sequence: SequenceNum,
    /// Sender correlation identifier; zero means no correlation was supplied.
    pub action_id: u32,
}

/// Progress of an adapter-owned flight procedure, independent of FC state.
///
/// This reports the adapter's procedure, not a fresh vehicle measurement.
/// Re-publication preserves the revision and transition time. Completion
/// needs an observation that satisfies the procedure's completion condition;
/// a command acknowledgement or elapsed timeout cannot establish completion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlightProcedureSample {
    /// Original request identity; it does not change on authority loss.
    pub request: FlightProcedureRequest,
    /// Opaque identity of this adapter attachment.
    pub adapter_incarnation: SourceIncarnation,
    /// Wrapping revision within this request, advanced only on a status change.
    pub revision: u64,
    /// Last status transition, on the monotonic clock since adapter start.
    pub changed_at: MonoTimestamp,
    /// Procedure requested by the operator.
    pub kind: FlightProcedureKind,
    /// Progress established by the adapter's execution and observation path.
    pub phase: FlightProcedurePhase,
    /// Typed cause when the procedure cannot continue or complete.
    pub reason: Option<FlightProcedureReason>,
    /// FC acknowledgement when observed for this procedure's command.
    pub fc_outcome: Option<CommandOutcome>,
}
