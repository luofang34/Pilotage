//! Whether the vehicle left flight on its own: the flight controller's
//! stamped arm and landed reports, read for a disarm after an arm or a
//! landing after the vehicle was in the air. A plan never lands, so either
//! one during the flown phases ends the run with its reason.

use pilotage_protocol::wire;

/// What the flight controller has reported during this run.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub(super) struct FlightWatch {
    armed: bool,
    in_air: bool,
    /// The flight controller has sent a stamped state report in this run.
    reports: bool,
    /// The arm that waits for the flight controller's answer.
    pub(super) arm: super::arm_receipt::ArmReceipt,
}

impl FlightWatch {
    /// Takes one report; the reason the run must end when it shows the
    /// vehicle leaving the flight it was in.
    pub(super) fn observe(&mut self, fc: &wire::FcState) -> Option<&'static str> {
        self.reports |= fc.stamp.is_some();
        if fc.stamp.is_some() {
            match fc.arm_state {
                2 => self.armed = true,
                1 if self.armed => return Some("the vehicle disarmed outside the plan"),
                _ => {}
            }
        }
        let landed = fc
            .landed_state
            .as_ref()
            .filter(|landed| landed.stamp.is_some())
            .and_then(|landed| wire::LandedState::try_from(landed.state).ok());
        match landed {
            Some(wire::LandedState::InAir) => self.in_air = true,
            Some(wire::LandedState::Landing | wire::LandedState::OnGround) if self.in_air => {
                return Some("the vehicle landed outside the plan");
            }
            _ => {}
        }
        None
    }
}

impl FlightWatch {
    /// Whether the flight controller reports its state, so that its answer
    /// to an arm can arrive. A vehicle with no flight-controller state
    /// report (the reference skiff) has no answer but the command path.
    pub(super) const fn reports_state(&self) -> bool {
        self.reports
    }
}

#[cfg(test)]
mod tests;
