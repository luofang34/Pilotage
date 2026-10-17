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
//!
//! An order is never left waiting in silence either: the flight
//! controller's own refusal answers it, and an order nothing answers
//! within [`ANSWER_WINDOW_MS`] snaps back to SAFE saying so, and is
//! still answered if the vehicle carries it out late.

/// How long an order waits for the vehicle's answer. A PX4 offboard
/// arm streams setpoints for a moment before it commands the arm.
pub const ANSWER_WINDOW_MS: f64 = 5000.0;

/// `MAV_RESULT_IN_PROGRESS`: the command is not answered yet.
const MAV_RESULT_IN_PROGRESS: u32 = 5;

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
    /// The FC's last acknowledgement, as (command kind, `MAV_RESULT`).
    verdict: Option<(u32, u32)>,
    /// The acknowledgement standing when the last order went out: a
    /// verdict equal to it answers an earlier order, not this one.
    verdict_at_order: Option<(u32, u32)>,
    /// When the clock first saw the order awaiting its answer.
    awaiting_since_ms: Option<f64>,
    /// An order that went unanswered in the window: a late answer from
    /// the FC still answers it, as a starved link can delay a real arm.
    unanswered: Option<ArmOrder>,
}

impl ArmTelegraph {
    /// Moves the lever. Returns the one command this move sends, or
    /// `None` when the vehicle already answers the ordered state — an
    /// idempotent move is a display act, not a command.
    pub fn set_order(&mut self, order: ArmOrder) -> Option<OrderAction> {
        self.order = order;
        self.phase = TelegraphPhase::InSync;
        self.verdict_at_order = self.verdict;
        self.awaiting_since_ms = None;
        self.unanswered = None;
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
        if let Some(late) = self.unanswered.take() {
            if matches!(
                (late, confirmed),
                (ArmOrder::Armed, ArmConfirmed::Armed) | (ArmOrder::Safe, ArmConfirmed::Disarmed)
            ) {
                // The vehicle did what was ordered, only late: the lever
                // shows it, so the screen never reads SAFE over a live
                // vehicle. Nothing is sent.
                self.order = late;
                self.phase = TelegraphPhase::InSync;
                return;
            }
            self.unanswered = Some(late);
        }
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

    /// Feeds the FC's acknowledgement of the last arm or disarm it was
    /// sent (`FcState.last_command_kind`: 0 none, 1 arm, 2 disarm;
    /// `last_command_result`: the raw `MAV_RESULT`). A refusal of the
    /// awaited command, newer than the order, answers it: the lever
    /// snaps back to SAFE with the FC's reason.
    pub fn on_fc_command_verdict(&mut self, kind: u32, result: u32) {
        if kind == 0 {
            return;
        }
        self.verdict = Some((kind, result));
        if self.verdict != self.verdict_at_order {
            self.answer_with_refusal();
        }
    }

    /// Advances the telegraph's clock. An order still unanswered after
    /// [`ANSWER_WINDOW_MS`] is answered by the FC's standing refusal of
    /// that command when there is one (a repeated refusal reads the same
    /// as the last), and otherwise snaps back to SAFE as unanswered.
    pub fn on_tick(&mut self, now_ms: f64) {
        if !matches!(self.phase, TelegraphPhase::AwaitingAnswer) {
            self.awaiting_since_ms = None;
            return;
        }
        let since = *self.awaiting_since_ms.get_or_insert(now_ms);
        if now_ms - since < ANSWER_WINDOW_MS {
            return;
        }
        if !self.answer_with_refusal() {
            self.unanswered = Some(self.order);
            self.order = ArmOrder::Safe;
            self.phase = TelegraphPhase::Refused("no answer from the vehicle".to_owned());
        }
    }

    /// Answers an awaited order with the FC's last verdict when that
    /// verdict refuses the same command; whether it did.
    fn answer_with_refusal(&mut self) -> bool {
        if !matches!(self.phase, TelegraphPhase::AwaitingAnswer) {
            return false;
        }
        let awaited = match self.order {
            ArmOrder::Armed => 1,
            ArmOrder::Safe => 2,
        };
        let Some((kind, result)) = self.verdict else {
            return false;
        };
        if kind != awaited || result == 0 || result == MAV_RESULT_IN_PROGRESS {
            return false;
        }
        self.order = ArmOrder::Safe;
        self.phase = TelegraphPhase::Refused(fc_refusal(result));
        true
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

/// The FC's reason for refusing, by `MAV_RESULT`, in the operator's words.
fn fc_refusal(result: u32) -> String {
    match result {
        1 => "the flight controller is not ready (temporarily rejected; \
              its preflight checks are the usual cause)"
            .to_owned(),
        2 => "the flight controller denied it".to_owned(),
        3 => "the flight controller does not support it".to_owned(),
        4 => "the flight controller failed to carry it out".to_owned(),
        6 => "the flight controller cancelled it".to_owned(),
        other => format!("the flight controller refused it (MAV_RESULT {other})"),
    }
}

#[cfg(test)]
mod tests;
