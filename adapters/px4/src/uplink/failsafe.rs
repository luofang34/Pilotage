//! The offboard-loss hold the multicopter uplink depends on: written at
//! connect under [`crate::config::Px4Profile::Simulation`], only read on a
//! physical vehicle ([`crate::pinning`]), and arming waits until PX4
//! reports it.
//!
//! The uplink's link-loss enactment stops the offboard stream, so PX4's own
//! offboard-loss failsafe takes the vehicle after `COM_OF_LOSS_T`. Its
//! default action (Position mode) needs RC, which a SITL vehicle flown over
//! MAVLink has not got, and PX4 falls back to Return and Land: the quad
//! leaves its position and descends. Pinned to Hold, a lost or latched
//! offboard stream keeps the vehicle where it is, as the other adapters'
//! neutral holds do.

use std::time::Instant;

use pilotage_mavlink::codec::{FcMessage, ParamValue};

use super::Px4Uplink;

/// `COM_OBL_RC_ACT = 5`: the offboard-loss failsafe is Hold.
pub(crate) const PINNED: [(&str, ParamValue); 1] = [("COM_OBL_RC_ACT", ParamValue::Int32(5))];

impl Px4Uplink {
    /// Sends the parameter requests that are due: writes under
    /// simulation, reads on a physical vehicle ([`crate::pinning`]).
    pub(super) fn pin_failsafe(&mut self, now: Instant) {
        for request in self.failsafe.due(now) {
            let frame = request.encode(
                self.seq,
                self.expected_system_id,
                self.expected_component_id,
            );
            self.send(&frame);
        }
    }

    /// Folds a PARAM_VALUE echo in; reports whether `message` was one.
    pub(super) fn observe_failsafe_echo(&mut self, message: &FcMessage) -> bool {
        if let FcMessage::ParamValue { param_id, value } = *message {
            self.failsafe.observe(param_id, value);
            return true;
        }
        false
    }

    /// Why arming waits on the offboard-loss Hold, if it does.
    pub(crate) fn failsafe_refusal(&self) -> Option<crate::pinning::ParamRefusal> {
        self.failsafe.refusal()
    }
}

#[cfg(test)]
impl Px4Uplink {
    /// Folds in PX4's echo of every pinned write, for tests.
    pub(crate) fn confirm_failsafe(&mut self) {
        for (name, value) in PINNED {
            self.failsafe
                .observe(pilotage_mavlink::codec::param_id(name), value);
        }
    }
}
