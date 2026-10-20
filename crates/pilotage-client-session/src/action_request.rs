//! Request identity from a prepared reliable control action.

use crate::ClientAction;

/// The authority binding encoded in one reliable control action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionRequestIdentity {
    /// Session that prepared the action.
    pub session_id: u64,
    /// Vehicle that receives the action.
    pub vehicle_id: u64,
    /// Control scope of the active lane.
    pub scope: String,
    /// Authority generation of the active lane.
    pub generation: u64,
    /// Host control-frame sequence; zero for a reliable action.
    pub sequence: u32,
    /// Nonzero correlation identifier encoded in the request.
    pub action_id: u32,
}

/// A reliable action prepared under an active lease.
///
/// This does not confirm a transport write or host acceptance. The port must
/// execute the actions in order. If the write fails, it must report
/// [`crate::TransportEvent::TransportLost`] and invalidate pending progress.
#[derive(Debug, Clone, PartialEq)]
pub struct PreparedControlAction {
    /// Identity encoded in the action's reliable command.
    pub identity: ActionRequestIdentity,
    /// Ordered work for the transport port, including the reliable command.
    pub actions: Vec<ClientAction>,
}
