//! One mode exchange shared by diagnostics and lease-bound motion.

mod admission;
mod progress;
mod retention;
mod status;

pub use status::{
    DjiVirtualStickDiagnosticError, DjiVirtualStickDiagnosticPhase,
    DjiVirtualStickDiagnosticReason, DjiVirtualStickDiagnosticStatus, DjiVirtualStickModeEvidence,
};

use std::time::Duration;

use pilotage_adapter_api::SourceIncarnation;

use crate::flight::{VirtualStickMode, VirtualStickModePacket, VirtualStickReport};

use DjiVirtualStickDiagnosticPhase as Phase;
use DjiVirtualStickDiagnosticReason as Reason;

const MAX_SEND_DELAY: Duration = Duration::from_millis(250);
const REPORT_MAX_AGE: Duration = Duration::from_millis(1500);
const ENABLE_OBSERVATION_BUDGET: Duration = Duration::from_millis(1250);
const CLEANUP_SEND_BUDGET: Duration = Duration::from_millis(1500);
const CLEANUP_OBSERVATION_BUDGET: Duration = Duration::from_millis(1500);

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::adapter) struct ModeFence {
    request_id: u64,
    incarnation: SourceIncarnation,
    pub(in crate::adapter) model_revision: u64,
    pub(in crate::adapter) mode: VirtualStickMode,
    sequence: u16,
    pub(in crate::adapter) send_until: Duration,
}

struct Diagnostic {
    status: DjiVirtualStickDiagnosticStatus,
    enable: ModeFence,
    disable: ModeFence,
    enable_report_sequence: u16,
    disable_report_sequence: Option<u16>,
    disable_queued: bool,
    retain_until: Option<Duration>,
}

impl Diagnostic {
    fn new(enable: ModeFence, disable: ModeFence, report_sequence: u16) -> Self {
        Self {
            status: DjiVirtualStickDiagnosticStatus {
                request_id: enable.request_id,
                incarnation: enable.incarnation,
                model_revision: enable.model_revision,
                phase: Phase::Enable,
                enable: DjiVirtualStickModeEvidence {
                    sequence: enable.sequence,
                    ..Default::default()
                },
                disable: DjiVirtualStickModeEvidence {
                    sequence: disable.sequence,
                    ..Default::default()
                },
                enable_send_until: enable.send_until,
                enable_observe_until: enable.send_until.saturating_add(ENABLE_OBSERVATION_BUDGET),
                disable_send_until: disable.send_until,
                complete_until: disable
                    .send_until
                    .saturating_add(CLEANUP_OBSERVATION_BUDGET),
                reason: None,
            },
            enable,
            disable,
            enable_report_sequence: report_sequence,
            disable_report_sequence: None,
            disable_queued: false,
            retain_until: None,
        }
    }
}

#[derive(Clone, Copy)]
struct ModeReport {
    report: VirtualStickReport,
    received: Duration,
    model_revision: u64,
}

pub(in crate::adapter) struct ModeSession {
    incarnation: SourceIncarnation,
    latest: Option<ModeReport>,
    last_sequence: Option<u16>,
    diagnostic: Option<Diagnostic>,
}

impl ModeSession {
    pub(in crate::adapter) fn new(incarnation: SourceIncarnation) -> Self {
        Self {
            incarnation,
            latest: None,
            last_sequence: None,
            diagnostic: None,
        }
    }

    pub(in crate::adapter) fn status_cached(&self) -> Option<DjiVirtualStickDiagnosticStatus> {
        self.diagnostic.as_ref().map(|diagnostic| diagnostic.status)
    }

    pub(in crate::adapter) fn active(&self) -> bool {
        self.diagnostic
            .as_ref()
            .is_some_and(|diagnostic| diagnostic.status.phase != Phase::Finished)
    }

    pub(in crate::adapter) fn unresolved(&self) -> bool {
        self.diagnostic.as_ref().is_some_and(|diagnostic| {
            diagnostic.status.enable.write_started
                && !(diagnostic.status.disable.acknowledgement == Some(0)
                    && diagnostic.status.disable.observed_at.is_some())
        })
    }

    pub(in crate::adapter) fn begin(
        &mut self,
        request_id: u64,
        model_revision: u64,
        sequence: u16,
        now: Duration,
        send_until: Duration,
    ) -> Result<ModeFence, DjiVirtualStickDiagnosticError> {
        if self.diagnostic.is_some() {
            return Err(DjiVirtualStickDiagnosticError::AlreadyAttempted);
        }
        if request_id == 0 || send_until <= now || send_until - now > MAX_SEND_DELAY {
            return Err(DjiVirtualStickDiagnosticError::InvalidDeadline);
        }
        let report = self
            .report_cached(now, model_revision)
            .ok_or(DjiVirtualStickDiagnosticError::MissingFeedback)?;
        if report.report.enabled
            || !report.report.multiple_flight_modes
            || report.report.novice_mode
        {
            return Err(DjiVirtualStickDiagnosticError::Configuration);
        }
        let enable = ModeFence {
            request_id,
            incarnation: self.incarnation,
            model_revision,
            mode: VirtualStickMode::Enable,
            sequence,
            send_until,
        };
        let disable = ModeFence {
            mode: VirtualStickMode::Disable,
            sequence: sequence.wrapping_add(1),
            send_until: send_until.saturating_add(CLEANUP_SEND_BUDGET),
            ..enable
        };
        self.diagnostic = Some(Diagnostic::new(enable, disable, report.report.sequence));
        Ok(enable)
    }

    fn report_cached(&self, now: Duration, model_revision: u64) -> Option<ModeReport> {
        self.latest.filter(|latest| {
            latest.model_revision == model_revision
                && now
                    .checked_sub(latest.received)
                    .is_some_and(|age| age < REPORT_MAX_AGE)
        })
    }

    pub(in crate::adapter) fn permits(&self, fence: &ModeFence, now: Duration) -> bool {
        let Some(diagnostic) = &self.diagnostic else {
            return false;
        };
        let (phase, expected) = match fence.mode {
            VirtualStickMode::Enable => (Phase::Enable, diagnostic.enable),
            VirtualStickMode::Disable => (Phase::Disable, diagnostic.disable),
        };
        let guarded = fence.mode == VirtualStickMode::Disable
            || self
                .report_cached(now, fence.model_revision)
                .is_some_and(|latest| {
                    !latest.report.enabled
                        && latest.report.multiple_flight_modes
                        && !latest.report.novice_mode
                });
        diagnostic.status.phase == phase && *fence == expected && now < fence.send_until && guarded
    }

    pub(in crate::adapter) fn packet(fence: &ModeFence) -> VirtualStickModePacket {
        fence.mode.packet(fence.sequence)
    }

    pub(in crate::adapter) fn transport_failed(&mut self) {
        if let Some(diagnostic) = &mut self.diagnostic
            && diagnostic.status.phase != Phase::Finished
        {
            diagnostic.status.reason = Some(Reason::TransportUncertain);
            diagnostic.status.phase = Phase::Finished;
        }
    }

    pub(in crate::adapter) fn source_changed(&mut self) {
        self.latest = None;
        if let Some(diagnostic) = &mut self.diagnostic
            && diagnostic.status.phase != Phase::Finished
        {
            diagnostic.status.reason = Some(Reason::SourceChanged);
            diagnostic.status.phase = Phase::Finished;
        }
    }
}
