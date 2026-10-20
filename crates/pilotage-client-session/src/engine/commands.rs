//! Fenced command construction and host-issued validity windows.

use super::*;
use crate::PreparedControlAction;

impl ClientEngine {
    /// Builds the next fenced control-frame datagram on the named lane,
    /// or nothing when that lease is not held — a shell cannot send
    /// unfenced input, and a frame must never ride a sibling's fencing.
    pub fn control_frame(
        &mut self,
        vehicle_id: u64,
        scope: &str,
        command: ControlCommand,
        sampled_at_nanos: u64,
    ) -> Vec<ClientAction> {
        if !self.holds(vehicle_id, scope) {
            return Vec::new();
        }
        self.last_control_at = sampled_at_nanos;
        let mut actions = self.probe_control_window(sampled_at_nanos);
        let required = self.requires_control_window();
        if let Some(lane) = self.lanes.get_mut(&(vehicle_id, scope.to_owned())) {
            lane.set_control_window(self.window.id_at(sampled_at_nanos), required);
            actions.push(ClientAction::SendDatagram(
                lane.frame(command, sampled_at_nanos),
            ));
        }
        actions
    }

    /// Builds a reliable discrete-action command under the named lease,
    /// correlated in the client's one id space regardless of lane.
    pub fn control_action(
        &mut self,
        vehicle_id: u64,
        scope: &str,
        request: wire::ControlActionRequest,
    ) -> Vec<ClientAction> {
        self.control_action_at(vehicle_id, scope, request, self.last_control_at)
    }

    /// Build a reliable action with the time of its original input event.
    pub fn control_action_at(
        &mut self,
        vehicle_id: u64,
        scope: &str,
        request: wire::ControlActionRequest,
        sampled_at_nanos: u64,
    ) -> Vec<ClientAction> {
        self.prepare_control_action_at(vehicle_id, scope, request, sampled_at_nanos)
            .map_or_else(Vec::new, |prepared| prepared.actions)
    }

    /// Prepares an action and its exact request identity from the active lane.
    ///
    /// Returns `None` when the lease or required validity window is absent.
    /// No failed input is stored for later replay. A prepared action does not
    /// confirm a transport write, host acceptance, or procedure completion.
    pub fn prepare_control_action_at(
        &mut self,
        vehicle_id: u64,
        scope: &str,
        request: wire::ControlActionRequest,
        sampled_at_nanos: u64,
    ) -> Option<PreparedControlAction> {
        if !self.control_action_ready(vehicle_id, scope, sampled_at_nanos) {
            return None;
        }
        let mut actions = self.probe_control_window(sampled_at_nanos);
        let required = self.requires_control_window();
        let lane = self.lanes.get_mut(&(vehicle_id, scope.to_owned()))?;
        let mut request = request;
        if request.action_id == 0 {
            self.next_action_id = self.next_action_id.wrapping_add(1).max(1);
            request.action_id = self.next_action_id;
        }
        lane.set_control_window(self.window.id_at(sampled_at_nanos), required);
        let (identity, bytes) = lane.prepare_action_at(request, sampled_at_nanos);
        actions.push(ClientAction::SendBootstrap(bytes));
        Some(PreparedControlAction { identity, actions })
    }

    pub(super) fn requires_control_window(&self) -> bool {
        self.admission
            .as_ref()
            .is_some_and(|admission| admission.physically_embodied)
    }
}
