use std::time::Duration;

use pilotage_adapter_api::FlightProcedureReason;

use super::Connection;
use crate::{DumlFrame, RouteAddress};

const MAX_AGE: Duration = Duration::from_millis(500);
const CENTER: u16 = 1024;
const CENTER_TOLERANCE: u16 = 32;

#[derive(Default)]
pub(super) struct RemoteControl {
    sequence: Option<u16>,
    latest: Option<(Duration, bool)>,
}

impl RemoteControl {
    fn receive(&mut self, frame: &DumlFrame, now: Duration) -> Option<bool> {
        if frame.sender() != RouteAddress(6)
            || frame.receiver() != RouteAddress(2)
            || frame.flags() != 0
            || frame.command_set() != 6
            || frame.command_id() != 5
        {
            return None;
        }
        // RC-N1 unsolicited pushes use zero for every frame. Their receipt time
        // bounds monitoring freshness; that field cannot order these reports.
        if frame.sequence() != 0 {
            if self.sequence.is_some_and(|previous| {
                let delta = frame.sequence().wrapping_sub(previous);
                delta == 0 || delta >= 0x8000
            }) {
                return None;
            }
            self.sequence = Some(frame.sequence());
        }
        self.latest = None;
        let [a, b, c, d, e, f, g, h, _, _, _, _, _] = frame.payload() else {
            return Some(false);
        };
        // RC channel-param pushes carry four little-endian stick channels.
        // SDK RCStick getters subtract 1024; the dial and buttons are separate.
        let axes = [[*a, *b], [*c, *d], [*e, *f], [*g, *h]].map(u16::from_le_bytes);
        if axes.iter().any(|value| !(364..=1684).contains(value)) {
            return Some(false);
        }
        let neutral = axes
            .iter()
            .all(|value| value.abs_diff(CENTER) <= CENTER_TOLERANCE);
        self.latest = Some((now, neutral));
        Some(neutral)
    }

    pub(super) fn neutral_cached(&self, now: Duration) -> bool {
        self.latest.is_some_and(|(received, neutral)| {
            neutral && now.checked_sub(received).is_some_and(|age| age < MAX_AGE)
        })
    }
}

impl Connection {
    pub(in crate::adapter) fn remote_control_neutral_cached(&self, now: Duration) -> bool {
        self.remote_control.neutral_cached(now)
    }

    pub(super) fn observe_remote_control(&mut self, frame: &DumlFrame, now: Duration) {
        if self.remote_control.receive(frame, now) == Some(false) {
            // Act on every frame, so a centered tail in the same receive batch
            // cannot erase the operator's request to take over.
            self.yield_to_remote(now);
        }
    }

    pub(in crate::adapter) fn guard_remote_control(&mut self, now: Duration) {
        let mode_lost = self.mode_session.status_cached().is_some_and(|status| {
            status.reason
                == Some(
                    crate::adapter::virtual_stick::DjiVirtualStickDiagnosticReason::SourceChanged,
                )
        });
        if mode_lost || (self.mode_session.active() && !self.remote_control.neutral_cached(now)) {
            self.yield_to_remote(now);
        }
    }

    fn yield_to_remote(&mut self, now: Duration) {
        let acquiring_or_enabled = self.mode_session.status_cached().is_some_and(|status| {
            matches!(
                status.phase,
                crate::adapter::virtual_stick::DjiVirtualStickDiagnosticPhase::Enable
                    | crate::adapter::virtual_stick::DjiVirtualStickDiagnosticPhase::Enabled
            )
        });
        if !acquiring_or_enabled && !self.flight.holds_authority() && self.motion.stopped {
            return;
        }
        tracing::warn!(
            "DJI RC input or mode feedback requires handover; revoke software control and request mode release"
        );
        if let Some(binding) = self.motion.binding {
            self.flight.revoke_motion_authority(binding);
        } else {
            self.flight.revoke_device_authority();
        }
        self.motion.stop(now);
        self.cancel_stick_pending(now);
        self.cancel_movement();
        self.invalidate_flight(FlightProcedureReason::AuthorityLost, now);
        self.mode_session.release_retained(
            now,
            Some(crate::adapter::virtual_stick::DjiVirtualStickDiagnosticReason::Cancelled),
        );
    }
}
