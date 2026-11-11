//! Per-class drop accounting for the engine actor's fan-out (ADR-0009:
//! drops are counted, never silent; ADR-0011: counted per class).

use pilotage_host_core::engine_actor::MessageClass;

/// Wrapping per-class counters for messages dropped because a client's
/// outbound queue could not accept them without blocking the actor task.
#[derive(Debug, Default)]
pub(super) struct DropCounters {
    unicast: u64,
    authority_broadcast: u64,
    telemetry: u64,
}

impl DropCounters {
    pub(super) fn record(&mut self, class: MessageClass) -> u64 {
        let counter = match class {
            MessageClass::Unicast => &mut self.unicast,
            MessageClass::AuthorityBroadcast => &mut self.authority_broadcast,
            MessageClass::Telemetry => &mut self.telemetry,
        };
        *counter = counter.wrapping_add(1);
        *counter
    }
}
