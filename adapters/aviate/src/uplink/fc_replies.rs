//! FC reply drainage for the flight uplink: COMMAND_ACK results and the
//! heartbeats the FC sends its learned commander, filtered to the
//! expected MAVLink source identity.

use tracing::{info, warn};

use super::FlightUplink;

impl FlightUplink {
    /// Drains FC replies off the uplink transport (COMMAND_ACK, heartbeats
    /// the FC sends its learned commander), returning the latest
    /// armed-state report if any heartbeat arrived. Non-blocking; call
    /// from the sampling tick.
    pub fn poll_fc(&mut self) -> Option<bool> {
        let mut messages: Vec<(
            pilotage_mavlink::codec::FrameSource,
            pilotage_mavlink::codec::FcMessage,
        )> = Vec::new();
        let mut armed: Option<bool> = None;
        while let Some(datagram) = self.transport.receive() {
            messages.clear();
            pilotage_mavlink::codec::parse_datagram(&datagram, &mut messages);
            for (source, message) in &messages {
                let (system_id, component_id) = self.policy.target();
                if source.system_id != system_id || source.component_id != component_id {
                    continue;
                }
                match *message {
                    pilotage_mavlink::codec::FcMessage::Heartbeat { armed: a, .. } => {
                        armed = Some(a)
                    }
                    pilotage_mavlink::codec::FcMessage::CommandAck {
                        command,
                        result,
                        target_system,
                        target_component,
                    } => {
                        if result == 0 {
                            info!(command, "FC accepted command");
                        } else {
                            warn!(command, result, "FC rejected command");
                        }
                        if command == pilotage_mavlink::codec::MAV_CMD_COMPONENT_ARM_DISARM
                            && addressed_to_gcs(target_system, target_component)
                            && let Some(arm) = self.arm_sent
                        {
                            self.arm_ack = Some(pilotage_adapter_api::FcCommandAck {
                                arm,
                                outcome: pilotage_mavlink::command_outcome(result),
                            });
                        }
                    }
                    _ => {}
                }
            }
        }
        armed
    }
}

/// Whether an ack addresses this GCS (system 255, component 190). In
/// MAVLink a zero target system addresses every system and a zero target
/// component addresses every component of the system.
fn addressed_to_gcs(target_system: u8, target_component: u8) -> bool {
    matches!(target_system, 0 | pilotage_mavlink::codec::GCS_SYSTEM_ID)
        && matches!(
            target_component,
            0 | pilotage_mavlink::codec::GCS_COMPONENT_ID
        )
}

#[cfg(test)]
mod tests;
