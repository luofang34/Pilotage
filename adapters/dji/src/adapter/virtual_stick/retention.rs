use std::time::Duration;

use super::{CLEANUP_OBSERVATION_BUDGET, MAX_SEND_DELAY, ModeSession, Phase, Reason};

impl ModeSession {
    pub(in crate::adapter) fn reset_released(&mut self) -> bool {
        if self.active() || self.unresolved() {
            return false;
        }
        self.diagnostic = None;
        true
    }

    pub(in crate::adapter) fn retain(&mut self, now: Duration, input_until: Duration) -> bool {
        let Some(exchange) = &mut self.diagnostic else {
            return false;
        };
        if !matches!(exchange.status.phase, Phase::Enable | Phase::Enabled)
            || input_until <= now
            || input_until - now > MAX_SEND_DELAY
            || exchange.retain_until.is_some_and(|until| now >= until)
        {
            return false;
        }
        // Leave one bounded interval for the stream's neutral write. The stream
        // uses the original input deadline for every nonzero packet.
        exchange.retain_until = Some(input_until.saturating_add(MAX_SEND_DELAY));
        true
    }

    pub(in crate::adapter) fn retained(&self) -> bool {
        self.diagnostic
            .as_ref()
            .is_some_and(|exchange| exchange.retain_until.is_some())
    }

    pub(in crate::adapter) fn enabled(&self) -> bool {
        self.diagnostic
            .as_ref()
            .is_some_and(|exchange| exchange.status.phase == Phase::Enabled)
    }

    pub(in crate::adapter) fn release_retained(&mut self, now: Duration, reason: Option<Reason>) {
        let Some(exchange) = &mut self.diagnostic else {
            return;
        };
        if !matches!(exchange.status.phase, Phase::Enable | Phase::Enabled) {
            return;
        }
        if exchange.retain_until.is_some() {
            exchange.disable.send_until = now.saturating_add(MAX_SEND_DELAY);
            exchange.status.disable_send_until = exchange.disable.send_until;
            exchange.status.complete_until = exchange
                .disable
                .send_until
                .saturating_add(CLEANUP_OBSERVATION_BUDGET);
        }
        self.cleanup(reason);
    }
}
