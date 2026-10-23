//! Hold, resume and abort from the mission lane (ADR-0052). A hold keeps
//! the motion lease and frames zero velocity, so the vehicle stops in
//! place under the same fence; the sequencing clock pauses. An abort stops
//! the vehicle the same way for a short while, then releases the lease and
//! ends the run.

use navigate_contract::MonotonicNanos;
use pilotage_protocol::{ControlIntent, LeaseRelease, ReferenceFrame, ScopeId, VelocityIntent};
use pilotage_session::{ClientKey, DomainEnvelope};
use pilotage_timing::MonoTimestamp;
use tokio::sync::mpsc;
use tracing::info;

use super::{MISSION_SCOPE, MissionTask};
use crate::runtime::HOST_VEHICLE;

/// Zero-velocity frames an abort sends before it releases the lease.
const ABORT_STOP_FRAMES: u8 = 20;

/// One instruction from the mission lane to its run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RunControl {
    /// Stop in place and pause the sequencing clock.
    Hold,
    /// Continue a held run from its active leg.
    Resume,
    /// Stop in place, release the lease and end the run.
    Abort,
}

/// The next control, or never when the run has no lane.
pub(super) async fn next(controls: &mut Option<mpsc::Receiver<RunControl>>) -> RunControl {
    match controls.as_mut() {
        Some(receiver) => match receiver.recv().await {
            Some(control) => control,
            // The lane is gone: nothing steers this run any more, so it
            // stops the way an abort does.
            None => {
                *controls = None;
                RunControl::Abort
            }
        },
        None => std::future::pending().await,
    }
}

impl MissionTask {
    /// Binds the run to its lane: its principal key and its controls.
    pub(crate) fn bind(&mut self, client: ClientKey, controls: Option<mpsc::Receiver<RunControl>>) {
        self.client = client;
        self.controls = controls;
    }

    pub(super) fn on_control(&mut self, control: RunControl) {
        let now = MonotonicNanos::from_nanos(self.elapsed_nanos());
        match control {
            RunControl::Hold => {
                self.operator_hold = true;
                if let Some(mission) = self.mission.as_mut() {
                    mission.hold(now);
                }
            }
            RunControl::Resume => {
                if self.stopping.is_some() {
                    return;
                }
                self.operator_hold = false;
                if let Some(mission) = self.mission.as_mut() {
                    mission.resume(now);
                }
            }
            RunControl::Abort => {
                self.operator_hold = true;
                self.stopping.get_or_insert(ABORT_STOP_FRAMES);
                if let Some(mission) = self.mission.as_mut() {
                    mission.hold(now);
                }
            }
        }
        info!(?control, "mission run control");
        let held = self.operator_hold;
        self.update(|status| status.operator_held = held);
    }

    /// One tick of a held or stopping run: zero velocity under the held
    /// lease, then, for an abort, the release. Returns `false` when the
    /// run ends.
    pub(super) async fn hold_tick(&mut self) -> bool {
        let sampled = MonoTimestamp::from_nanos(self.elapsed_nanos());
        let stop = ControlIntent::Velocity(VelocityIntent {
            frame: ReferenceFrame::BodyFrd,
            vx: 0.0,
            vy: 0.0,
            vz: 0.0,
            yaw_rate: 0.0,
        });
        if self.generation.is_some() && !self.held && !self.send_intent(stop, sampled).await {
            return false;
        }
        let Some(remaining) = self.stopping else {
            return true;
        };
        if remaining > 0 && self.generation.is_some() {
            self.stopping = Some(remaining - 1);
            return true;
        }
        if self.generation.is_some() {
            let release = DomainEnvelope::Release(LeaseRelease {
                vehicle: HOST_VEHICLE,
                scope: ScopeId::new(MISSION_SCOPE),
            });
            self.send_message(release).await;
            self.generation = None;
        }
        self.fenced = true;
        self.update(|status| {
            status.stopped = true;
            status.fenced = true;
        });
        info!("mission run stopped and released the vehicle");
        false
    }
}
