//! A copy of an in-memory uplink, for the copy of an embedded adapter.

use super::FlightUplink;
use crate::error::AviateAdapterError;

impl FlightUplink {
    /// A second uplink in the same state, with its own queues.
    ///
    /// # Errors
    ///
    /// Returns [`AviateAdapterError::NotCopyable`] when the uplink owns a
    /// socket or reads the process clock.
    pub(crate) fn try_clone(&self) -> Result<Self, AviateAdapterError> {
        // Every field is named, so a new field does not compile until this
        // function copies it or refuses it.
        let Self {
            transport,
            target,
            policy,
            started,
            clock,
            send_failures,
            arm_sent,
            arm_ack,
            fc_mav_type,
            fc_system_status,
        } = self;
        let transport = transport
            .memory_copy()
            .ok_or(AviateAdapterError::NotCopyable {
                part: "the socket of the command link",
            })?;
        let clock = clock.host_set().ok_or(AviateAdapterError::NotCopyable {
            part: "the process clock",
        })?;
        Ok(Self {
            transport,
            target: *target,
            policy: policy.clone(),
            started: *started,
            clock,
            send_failures: *send_failures,
            arm_sent: *arm_sent,
            arm_ack: *arm_ack,
            fc_mav_type: *fc_mav_type,
            fc_system_status: *fc_system_status,
        })
    }
}
