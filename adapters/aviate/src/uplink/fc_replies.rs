//! FC reply drainage for the flight uplink: COMMAND_ACK results and the
//! heartbeats the FC sends its learned commander, filtered to the
//! expected MAVLink source identity.

use tracing::{info, warn};

use super::FlightUplink;

impl FlightUplink {
    /// Drains FC replies off the uplink socket (COMMAND_ACK, heartbeats
    /// the FC sends its learned commander), returning the latest
    /// armed-state report if any heartbeat arrived. Non-blocking; call
    /// from the sampling tick.
    pub fn poll_fc(&mut self) -> Option<bool> {
        let mut buf = [0u8; 512];
        let mut messages: Vec<(crate::mavlink::FrameSource, crate::mavlink::AviateMessage)> =
            Vec::new();
        let mut armed: Option<bool> = None;
        while let Ok((len, _)) = self.socket.recv_from(&mut buf) {
            messages.clear();
            crate::mavlink::parse_datagram(buf.get(..len).unwrap_or(&[]), &mut messages);
            for (source, message) in &messages {
                if source.system_id != self.expected_system_id
                    || source.component_id != self.expected_component_id
                {
                    continue;
                }
                match *message {
                    crate::mavlink::AviateMessage::Heartbeat { armed: a } => armed = Some(a),
                    crate::mavlink::AviateMessage::CommandAck { command, result } => {
                        if result == 0 {
                            info!(command, "FC accepted command");
                        } else {
                            warn!(command, result, "FC rejected command");
                        }
                    }
                    _ => {}
                }
            }
        }
        armed
    }
}
