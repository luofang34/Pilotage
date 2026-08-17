//! The arm order telegraph: one lever the operator sets, one lamp the
//! flight controller answers, and a machine that keeps the two honest.
//!
//! Arming is not a button press, it is an ORDER — the operator states an
//! intent (armed, or safe) and the vehicle answers through its own
//! state report, like an engine order telegraph answers the bridge. The
//! machine reconciles the two and shows the difference; it never hides
//! it behind a retry. Two safety rules are absolute:
//!
//! - No order is ever re-sent on its own. A refused order, and an order
//!   the vehicle unilaterally leaves (a failsafe disarm, a ground
//!   auto-disarm), snaps the lever back to SAFE with the reason shown.
//!   A vehicle that disarmed itself has a reason; re-arming it is a
//!   fresh human decision, never a reconciliation loop's.
//! - The confirmed lamp only ever repeats the flight controller's own
//!   report. An accepted command moves the lever, not the lamp.

/// What the operator's lever orders.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ArmOrder {
    /// Motors must not run.
    #[default]
    Safe,
    /// The vehicle is ordered live.
    Armed,
}

/// What the flight controller last reported.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ArmConfirmed {
    /// No report yet this session.
    #[default]
    Unknown,
    /// The FC reports disarmed.
    Disarmed,
    /// The FC reports armed.
    Armed,
}

/// Where the telegraph stands between order and answer.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum TelegraphPhase {
    /// Order and answer agree (or nothing was ever ordered).
    #[default]
    InSync,
    /// An order is out and the FC has not answered it yet.
    AwaitingAnswer,
    /// The host or vehicle refused the last order; the lever snapped
    /// back to SAFE and the reason stands until the next order.
    Refused(String),
    /// The vehicle left the ordered state on its own; the lever snapped
    /// back to SAFE. Re-arming is a fresh decision.
    Dropped,
}

/// The action a lever move asks the shell to send, in wire codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OrderAction {
    /// 1 arm, 2 disarm — `pilotage.v1.ControlAction`.
    pub action: u32,
}

/// The lever, the lamp, and the reconciliation between them.
#[derive(Debug, Default)]
pub struct ArmTelegraph {
    order: ArmOrder,
    confirmed: ArmConfirmed,
    phase: TelegraphPhase,
}

impl ArmTelegraph {
    /// Moves the lever. Returns the one command this move sends, or
    /// `None` when the vehicle already answers the ordered state — an
    /// idempotent move is a display act, not a command.
    pub fn set_order(&mut self, order: ArmOrder) -> Option<OrderAction> {
        self.order = order;
        self.phase = TelegraphPhase::InSync;
        let answered = matches!(
            (order, self.confirmed),
            (ArmOrder::Armed, ArmConfirmed::Armed) | (ArmOrder::Safe, ArmConfirmed::Disarmed)
        );
        if answered {
            return None;
        }
        self.phase = TelegraphPhase::AwaitingAnswer;
        Some(OrderAction {
            action: match order {
                ArmOrder::Armed => 1,
                ArmOrder::Safe => 2,
            },
        })
    }

    /// Feeds one arm/disarm action verdict. A refusal snaps the lever
    /// back to SAFE with the reason; an acceptance keeps waiting for
    /// the FC's own report — a verdict is not an answer.
    pub fn on_action_result(&mut self, action: u32, accepted: bool, detail: &str) {
        if action != 1 && action != 2 {
            return;
        }
        if !accepted {
            self.order = ArmOrder::Safe;
            self.phase = TelegraphPhase::Refused(if detail.is_empty() {
                "refused".to_owned()
            } else {
                detail.to_owned()
            });
        }
    }

    /// Feeds the FC's own arm report (`FcState.arm_state`: 1 disarmed,
    /// 2 armed). The lamp follows it verbatim; a vehicle that leaves an
    /// ordered ARM on its own snaps the lever back to SAFE and says so.
    pub fn on_fc_arm_state(&mut self, arm_state: u32) {
        let confirmed = match arm_state {
            1 => ArmConfirmed::Disarmed,
            2 => ArmConfirmed::Armed,
            _ => return,
        };
        let was = self.confirmed;
        self.confirmed = confirmed;
        match (self.order, confirmed) {
            (ArmOrder::Armed, ArmConfirmed::Armed) | (ArmOrder::Safe, ArmConfirmed::Disarmed) => {
                if matches!(self.phase, TelegraphPhase::AwaitingAnswer) {
                    self.phase = TelegraphPhase::InSync;
                }
            }
            (ArmOrder::Armed, ArmConfirmed::Disarmed) if was == ArmConfirmed::Armed => {
                self.order = ArmOrder::Safe;
                self.phase = TelegraphPhase::Dropped;
            }
            _ => {}
        }
    }

    /// A fresh session voids every order and every report.
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// The lever's position.
    #[must_use]
    pub fn order(&self) -> ArmOrder {
        self.order
    }

    /// The lamp's report.
    #[must_use]
    pub fn confirmed(&self) -> ArmConfirmed {
        self.confirmed
    }

    /// Where order and answer stand relative to each other.
    #[must_use]
    pub fn phase(&self) -> &TelegraphPhase {
        &self.phase
    }
}

#[cfg(test)]
mod tests;
