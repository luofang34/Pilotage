//! Receive, queue, and host-issued command validity.

use super::SessionEngine;
use crate::message::ClientKey;
use crate::{DomainEnvelope, SessionOutcome};
use pilotage_protocol::{AdmittedTiming, CommandTiming};
use pilotage_timing::MonoTimestamp;

impl SessionEngine {
    /// Correlate source time at ingress, before the host actor queue.
    pub fn observe_receive_time(
        &mut self,
        client: ClientKey,
        sampled: MonoTimestamp,
        received: MonoTimestamp,
    ) {
        self.clients.correlated_age(client, received, sampled);
    }

    /// Handle a message with its receive time and current execution time.
    /// Queue residence consumes validity; it cannot establish a fresh clock floor.
    #[must_use]
    pub fn handle_received_message(
        &mut self,
        client: ClientKey,
        message: DomainEnvelope,
        received_at: MonoTimestamp,
        now: MonoTimestamp,
    ) -> SessionOutcome {
        let sampled = match &message {
            DomainEnvelope::Frame(frame) => Some(frame.sampled_at),
            DomainEnvelope::ActionCommand(command) if command.timing.is_some() => {
                Some(command.sampled_at)
            }
            _ => None,
        };
        if let Some(sampled) = sampled {
            self.clients.correlated_age(client, received_at, sampled);
        }
        self.handle_client_message(client, message, now)
    }

    pub(super) fn admit_timing(
        &mut self,
        client: ClientKey,
        sampled: MonoTimestamp,
        timing: CommandTiming,
        now: MonoTimestamp,
    ) -> Option<AdmittedTiming> {
        let relative_age = self.clients.correlated_age(client, now, sampled);
        let mut age = relative_age;
        let bounded = timing.require_control_window;
        if bounded {
            let window_age =
                self.clients
                    .control_window_age(client, timing.control_window_id, sampled, now)?;
            if window_age >= core::time::Duration::from_millis(250) {
                return None;
            }
            age = age.max(window_age);
        }
        let send_remaining = timing.max_delay.checked_sub(age)?;
        if send_remaining.is_zero() {
            return None;
        }
        let active_remaining = match timing.active_for {
            Some(lifetime) => {
                let remaining = lifetime.checked_sub(age)?;
                if remaining.is_zero() {
                    return None;
                }
                Some(remaining)
            }
            None => None,
        };
        let completion_timeout = match timing.completion_timeout {
            Some(timeout) => Some(timeout.checked_sub(age).filter(|value| !value.is_zero())?),
            None => None,
        };
        Some(AdmittedTiming {
            admitted_at: now,
            send_remaining,
            active_remaining,
            completion_timeout,
            window_bounded: bounded,
        })
    }
}
