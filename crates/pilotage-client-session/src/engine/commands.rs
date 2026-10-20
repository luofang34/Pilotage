//! Fenced command construction and host-issued validity windows.

use super::*;

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
            lane.set_control_window(
                if sampled_at_nanos >= self.window_received_at_ns {
                    self.control_window_id
                } else {
                    0
                },
                required,
            );
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
        if !self.holds(vehicle_id, scope) {
            return Vec::new();
        }
        let mut actions = self.probe_control_window(sampled_at_nanos);
        let required = self.requires_control_window();
        let Some(lane) = self.lanes.get_mut(&(vehicle_id, scope.to_owned())) else {
            return Vec::new();
        };
        let mut request = request;
        if request.action_id == 0 {
            self.next_action_id = self.next_action_id.wrapping_add(1);
            request.action_id = self.next_action_id;
        }
        lane.set_control_window(
            if sampled_at_nanos >= self.window_received_at_ns {
                self.control_window_id
            } else {
                0
            },
            required,
        );
        actions.push(ClientAction::SendBootstrap(
            lane.action_command_at(request, sampled_at_nanos),
        ));
        actions
    }

    pub(super) fn requires_control_window(&self) -> bool {
        self.admission
            .as_ref()
            .is_some_and(|admission| admission.physically_embodied)
    }

    fn probe_control_window(&mut self, now_ns: u64) -> Vec<ClientAction> {
        if !self.requires_control_window()
            || self
                .window_probe_at
                .is_some_and(|previous| now_ns.saturating_sub(previous) < 100_000_000)
        {
            return Vec::new();
        }
        self.window_probe_at = Some(now_ns);
        use prost::Message;
        let envelope = wire::Envelope {
            schema_version: pilotage_protocol::SCHEMA_VERSION,
            payload: Some(wire::envelope::Payload::Ping(wire::Ping {
                nonce: now_ns,
                sender_sent_at: Some(wire::MonoTimestamp { nanos: now_ns }),
            })),
        };
        vec![ClientAction::SendDatagram(envelope.encode_to_vec())]
    }
}
