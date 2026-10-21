use std::time::Duration;

use super::{DjiVirtualStickModeEvidence, ModeFence, ModeReport, ModeSession, Phase, Reason};
use crate::{DumlFrame, flight::VirtualStickReport};

impl ModeSession {
    pub(in crate::adapter) fn receive(
        &mut self,
        frame: &DumlFrame,
        now: Duration,
        model_revision: Option<u64>,
    ) {
        self.response(frame, now);
        let Some(model_revision) = model_revision else {
            return;
        };
        let Some(report) = VirtualStickReport::from_push(frame) else {
            if frame.sender() == crate::RouteAddress(14)
                && frame.receiver() == crate::RouteAddress(2)
                && frame.flags() == 0
                && frame.command_set() == 3
                && frame.command_id() == 0x44
            {
                self.latest = None;
            }
            return;
        };
        if self
            .last_sequence
            .is_some_and(|last| !forward(report.sequence, last))
        {
            return;
        }
        self.last_sequence = Some(report.sequence);
        self.latest = Some(ModeReport {
            report,
            received: now,
            model_revision,
        });
        let Some(diagnostic) = &mut self.diagnostic else {
            return;
        };
        if diagnostic.enable.model_revision != model_revision {
            return;
        }
        let (evidence, expected, watermark) = match diagnostic.status.phase {
            Phase::Enable => (
                &mut diagnostic.status.enable,
                true,
                Some(diagnostic.enable_report_sequence),
            ),
            Phase::Disable => (
                &mut diagnostic.status.disable,
                false,
                diagnostic.disable_report_sequence,
            ),
            Phase::Enabled | Phase::Finished => return,
        };
        if evidence.written_at.is_some_and(|written| now > written)
            && watermark.is_some_and(|last| forward(report.sequence, last))
            && report.enabled == expected
            && (!expected || (report.multiple_flight_modes && !report.novice_mode))
        {
            evidence.observed_at = Some(now);
            evidence.observed_sequence = Some(report.sequence);
        }
    }

    fn response(&mut self, frame: &DumlFrame, now: Duration) {
        let Some(diagnostic) = &mut self.diagnostic else {
            return;
        };
        let (fence, evidence) = match diagnostic.status.phase {
            Phase::Enable => (diagnostic.enable, &mut diagnostic.status.enable),
            Phase::Disable => (diagnostic.disable, &mut diagnostic.status.disable),
            Phase::Enabled | Phase::Finished => return,
        };
        if evidence.written_at.is_none() || evidence.acknowledgement.is_some() {
            return;
        }
        if let Some(code) = Self::packet(&fence).response(frame) {
            evidence.acknowledgement = Some(code);
            evidence.acknowledged_at = Some(now);
            tracing::info!(request_id = fence.request_id, mode = ?fence.mode, code,
                "DJI mode diagnostic response; exclusive control is not established");
        }
    }

    pub(in crate::adapter) fn wrote(&mut self, fence: &ModeFence, now: Duration, recorded: bool) {
        let Some(diagnostic) = &mut self.diagnostic else {
            return;
        };
        let evidence = if *fence == diagnostic.enable {
            &mut diagnostic.status.enable
        } else if *fence == diagnostic.disable {
            &mut diagnostic.status.disable
        } else {
            return;
        };
        evidence.write_started = true;
        if recorded {
            evidence.written_at = Some(now);
        }
    }

    pub(in crate::adapter) fn poll(&mut self, now: Duration, model_revision: Option<u64>) {
        let current = model_revision.and_then(|revision| self.report_cached(now, revision));
        let Some(diagnostic) = &mut self.diagnostic else {
            return;
        };
        if diagnostic.status.phase == Phase::Finished {
            return;
        }
        if model_revision != Some(diagnostic.enable.model_revision) {
            self.source_changed();
            return;
        }
        if diagnostic.retain_until.is_some_and(|until| now >= until)
            && matches!(diagnostic.status.phase, Phase::Enable | Phase::Enabled)
        {
            self.release_retained(now, Some(Reason::Cancelled));
            return;
        }
        match diagnostic.status.phase {
            Phase::Enable => {
                let evidence = diagnostic.status.enable;
                let until = if evidence.written_at.is_some() {
                    diagnostic.status.enable_observe_until
                } else {
                    diagnostic.enable.send_until
                };
                if confirmed(evidence) && current.is_some_and(|latest| latest.report.enabled) {
                    if diagnostic.retain_until.is_some() {
                        diagnostic.status.phase = Phase::Enabled;
                    } else {
                        self.cleanup(None);
                    }
                } else if now >= until || evidence.acknowledgement.is_some_and(|code| code != 0) {
                    self.cleanup(Some(Reason::EnableUnconfirmed));
                }
            }
            Phase::Disable => {
                let until = if diagnostic.status.disable.written_at.is_some() {
                    diagnostic.status.complete_until
                } else {
                    diagnostic.disable.send_until
                };
                if confirmed(diagnostic.status.disable)
                    && current.is_some_and(|latest| !latest.report.enabled)
                {
                    diagnostic.status.phase = Phase::Finished;
                } else if now >= until
                    || diagnostic
                        .status
                        .disable
                        .acknowledgement
                        .is_some_and(|code| code != 0)
                {
                    diagnostic.status.reason = diagnostic
                        .status
                        .reason
                        .or(Some(Reason::DisableUnconfirmed));
                    diagnostic.status.phase = Phase::Finished;
                }
            }
            Phase::Enabled => {
                if current.is_none_or(|latest| {
                    !latest.report.enabled
                        || !latest.report.multiple_flight_modes
                        || latest.report.novice_mode
                }) {
                    self.release_retained(now, Some(Reason::SourceChanged));
                }
            }
            Phase::Finished => {}
        }
    }

    pub(in crate::adapter) fn cancel(&mut self) {
        if self.diagnostic.as_ref().is_some_and(|diagnostic| {
            matches!(diagnostic.status.phase, Phase::Enable | Phase::Enabled)
        }) {
            self.cleanup(Some(Reason::Cancelled));
        }
    }

    pub(super) fn cleanup(&mut self, reason: Option<Reason>) {
        let Some(diagnostic) = &mut self.diagnostic else {
            return;
        };
        diagnostic.status.reason = diagnostic.status.reason.or(reason);
        diagnostic.status.phase = if diagnostic.status.enable.write_started {
            Phase::Disable
        } else {
            Phase::Finished
        };
        diagnostic.disable_report_sequence = self.last_sequence;
    }

    pub(in crate::adapter) fn cleanup_fence(&mut self) -> Option<ModeFence> {
        let diagnostic = self.diagnostic.as_mut()?;
        if diagnostic.status.phase != Phase::Disable || diagnostic.disable_queued {
            return None;
        }
        diagnostic.disable_queued = true;
        Some(diagnostic.disable)
    }
}

fn confirmed(evidence: DjiVirtualStickModeEvidence) -> bool {
    evidence.acknowledgement == Some(0) && evidence.observed_at.is_some()
}

fn forward(sequence: u16, previous: u16) -> bool {
    let delta = sequence.wrapping_sub(previous);
    delta != 0 && delta < 0x8000
}
