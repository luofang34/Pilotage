use std::io;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, Instant};

use turret_core::Tick;
use turret_dji::{KeyframeState, KeyframeWriteError, KeyframeWriteReceipt, Mini3AccessoryKeyframe};

use super::{Connection, DjiFault, Framing};

const REQUEST_INTERVAL: Duration = Duration::from_secs(1);
const REQUEST_BUDGET: Duration = Duration::from_millis(500);

pub(in crate::adapter) struct Keyframes {
    pub(in crate::adapter) request: Arc<AtomicBool>,
    pub(super) active: Option<InFlight>,
    session: [u8; 16],
    next_request: Duration,
}

pub(super) struct InFlight {
    request: Mini3AccessoryKeyframe,
    send_before: Instant,
}

impl Keyframes {
    pub(super) fn new(session: [u8; 16]) -> Self {
        Self {
            request: Arc::new(AtomicBool::new(false)),
            active: None,
            session,
            next_request: Duration::ZERO,
        }
    }

    pub(super) fn close(&mut self) {
        if let Some(active) = &mut self.active {
            active.request.close();
        }
        self.active = None;
        self.request.store(false, Ordering::Release);
    }

    pub(super) fn preempt_for_movement(&mut self) -> Result<(), DjiFault> {
        let Some(active) = &mut self.active else {
            return Ok(());
        };
        match active.request.state() {
            KeyframeState::Partial => return Err(DjiFault::PartialCommand),
            KeyframeState::Aborted(reason) => return Err(DjiFault::Keyframe(reason)),
            KeyframeState::Waiting => {
                active.request.cancel();
                // Resume recovery with fresh sequences only after the existing interval.
                self.request.store(true, Ordering::Release);
            }
            KeyframeState::TransportAccepted(_) => {}
        }
        self.active = None;
        Ok(())
    }
}

impl Connection {
    pub(in crate::adapter) fn drive_keyframe_blocking(
        &mut self,
        sequence: &mut u16,
        now: Duration,
        clock: &mut impl FnMut() -> Duration,
    ) {
        if self.fault.is_some() || self.input.framing() != Framing::LogicLink {
            return;
        }
        if self.keyframes.active.is_none() {
            if self.pending.is_some()
                || now < self.keyframes.next_request
                || !self.keyframes.request.swap(false, Ordering::AcqRel)
            {
                return;
            }
            let until = now.saturating_add(REQUEST_BUDGET);
            let Some(send_before) = self.origin.checked_add(until) else {
                self.fail(DjiFault::Keyframe(turret_dji::KeyframeAbort::InvalidTiming));
                return;
            };
            match Mini3AccessoryKeyframe::new(
                self.keyframes.session,
                *sequence,
                tick(now),
                tick(now.saturating_add(REQUEST_BUDGET)),
            ) {
                Ok(active) => {
                    self.keyframes.active = Some(InFlight {
                        request: active,
                        send_before,
                    });
                    self.keyframes.next_request = now.saturating_add(REQUEST_INTERVAL);
                    *sequence = sequence.wrapping_add(3);
                }
                Err(error) => {
                    self.fail(DjiFault::Keyframe(error));
                    return;
                }
            }
        }
        self.write_keyframe_blocking(clock);
    }

    fn write_keyframe_blocking(&mut self, clock: &mut impl FnMut() -> Duration) {
        let Some(active) = &mut self.keyframes.active else {
            return;
        };
        let Some(transport) = &mut self.transport else {
            return;
        };
        let mut written = Vec::new();
        let mut completed = clock();
        let result =
            active
                .request
                .write_available(self.keyframes.session, tick(completed), |bytes| {
                    let count = match transport.write(bytes, active.send_before) {
                        Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
                        Ok(count) => count,
                        Err(error) if error.kind() == io::ErrorKind::WouldBlock => 0,
                        Err(error) => return Err(error),
                    };
                    completed = clock();
                    if let Some(actual) = bytes.get(..count) {
                        written.extend_from_slice(actual);
                    }
                    Ok(KeyframeWriteReceipt {
                        accepted: count,
                        completed_at: tick(completed),
                    })
                });
        let state = active.request.state();
        self.written_bytes = self.written_bytes.wrapping_add(written.len() as u64);
        if !written.is_empty() && !self.record_blocking(completed, "tx", &written) {
            return;
        }
        match result {
            Err(KeyframeWriteError::Transport(source)) => self.fail(DjiFault::Transport {
                operation: "request keyframe",
                source,
            }),
            Err(KeyframeWriteError::State(source)) => self.fail(DjiFault::Keyframe(source)),
            Ok(_) if matches!(state, KeyframeState::TransportAccepted(_)) => {
                self.keyframes.active = None;
                tracing::debug!("DJI keyframe request written; decoded picture remains unverified");
            }
            Ok(_) => {}
        }
    }
}

fn tick(time: Duration) -> Tick {
    Tick(time.as_micros().min(u128::from(u64::MAX)) as u64)
}
