use pilotage_adapter_api::SourceIncarnation;

/// Identity of the host's aircraft report stream across USB attachments.
///
/// Each attachment advances the epoch. Payload and command identities remain
/// separate. Retain this value only within the same vehicle and host session.
#[derive(Clone, Copy, Debug)]
pub struct DjiTelemetrySource {
    pub(super) incarnation: SourceIncarnation,
    pub(super) epoch: u32,
}

impl DjiTelemetrySource {
    pub(super) fn new(incarnation: SourceIncarnation) -> Self {
        Self {
            incarnation,
            epoch: 0,
        }
    }

    pub(super) fn next_attachment(self) -> Self {
        Self {
            epoch: self.epoch.wrapping_add(1),
            ..self
        }
    }
}
