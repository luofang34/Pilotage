//! Command validity independent of device execution completion.

use crate::{ConvertError, wire};
use core::time::Duration;

/// A sender's bounded command lifetime request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommandTiming {
    /// Maximum additional path delay and host queue delay.
    pub max_delay: Duration,
    /// Execution validity counted from the original sample; mandatory for continuous demands.
    pub active_for: Option<Duration>,
    /// Deadline budget to observe completion, counted from the original sample.
    pub completion_timeout: Option<Duration>,
    /// Host-issued validity window; zero means no window.
    pub control_window_id: u64,
    /// Reject if the host cannot validate the issued window.
    pub require_control_window: bool,
}

impl Default for CommandTiming {
    fn default() -> Self {
        Self {
            max_delay: Duration::from_millis(250),
            active_for: Some(Duration::from_millis(250)),
            completion_timeout: None,
            control_window_id: 0,
            require_control_window: false,
        }
    }
}

/// Remaining budgets at adapter delivery. A recipient consumes them once
/// into its local monotonic deadlines; forwarding must subtract elapsed time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AdmittedTiming {
    /// Host clock at which these remaining budgets were computed.
    pub admitted_at: pilotage_timing::MonoTimestamp,
    /// Time left to begin the transport write.
    pub send_remaining: Duration,
    /// Time left before execution validity expires.
    pub active_remaining: Option<Duration>,
    /// Time left to observe completion; queueing must not renew it.
    pub completion_timeout: Option<Duration>,
    /// A host-issued window bounded this admission on the host clock.
    pub window_bounded: bool,
}

impl From<CommandTiming> for wire::CommandTiming {
    fn from(timing: CommandTiming) -> Self {
        let millis = |value: Duration| u32::try_from(value.as_millis()).unwrap_or(u32::MAX);
        Self {
            max_delay_ms: millis(timing.max_delay),
            active_for_ms: timing.active_for.map_or(0, millis),
            completion_timeout_ms: timing.completion_timeout.map_or(0, millis),
            control_window_id: timing.control_window_id,
            require_control_window: timing.require_control_window,
        }
    }
}

impl TryFrom<wire::CommandTiming> for CommandTiming {
    type Error = ConvertError;

    fn try_from(timing: wire::CommandTiming) -> Result<Self, Self::Error> {
        if timing.max_delay_ms == 0
            || timing.max_delay_ms > 1000
            || timing.active_for_ms > 1000
            || timing.completion_timeout_ms > 60_000
        {
            return Err(ConvertError::MissingField {
                message: "pilotage.v1.CommandTiming",
                field: "bounded timing",
            });
        }
        let optional =
            |millis: u32| (millis > 0).then_some(Duration::from_millis(u64::from(millis)));
        Ok(Self {
            max_delay: Duration::from_millis(u64::from(timing.max_delay_ms)),
            active_for: optional(timing.active_for_ms),
            completion_timeout: optional(timing.completion_timeout_ms),
            control_window_id: timing.control_window_id,
            require_control_window: timing.require_control_window,
        })
    }
}
