//! `VehicleAdapter` implementation over the Aviate MAVLink link.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use pilotage_adapter_api::{
    AdapterCapabilities, ApplyOutcome, AvionicsSample, Disposition, ExecutionMode, LinkLossPolicy,
    Pose2d, RejectReason, StepBudget, StepOutcome, TelemetryBatch, TelemetrySample, VehicleAdapter,
    VehicleDescriptor, VideoSource,
};
use pilotage_protocol::{ScopedControlFrame, VehicleId};
use pilotage_timing::SimTick;

use crate::error::AviateAdapterError;
use crate::link::{AviateLink, LatestAviate, LinkConfig};

/// Data older than this is withheld from telemetry entirely, so
/// downstream freshness models see the group's age grow instead of a
/// frozen value replaying forever (the same withholding discipline as
/// the Gazebo adapter's dead-reader path).
const WITHHOLD_AFTER: Duration = Duration::from_secs(3);

/// Telemetry-only adapter for the Aviate flight controller (ADR-0018).
///
/// Real-time (ADR-0013): the FC advances on its own clock; `step`
/// reports the latest observed FC boot time as the simulation tick.
#[derive(Debug)]
pub struct AviateAdapter {
    vehicle: VehicleId,
    state: Arc<Mutex<LatestAviate>>,
    // Kept alive for its receive task; dropped with the adapter.
    _link: Option<AviateLink>,
    link_loss_policy: Option<LinkLossPolicy>,
}

impl AviateAdapter {
    /// Starts the MAVLink link and returns a ready adapter for `vehicle`.
    ///
    /// # Errors
    ///
    /// Returns [`AviateAdapterError`] when no UDP socket can be bound.
    pub async fn start(vehicle: VehicleId, config: LinkConfig) -> Result<Self, AviateAdapterError> {
        let link = AviateLink::start(config).await?;
        Ok(Self {
            vehicle,
            state: link.state(),
            _link: Some(link),
            link_loss_policy: None,
        })
    }

    /// Wires an adapter around a caller-supplied state cache, for tests.
    #[cfg(test)]
    pub(crate) fn from_state(vehicle: VehicleId, state: Arc<Mutex<LatestAviate>>) -> Self {
        Self {
            vehicle,
            state,
            _link: None,
            link_loss_policy: None,
        }
    }

    /// Yaw extracted from the body→NED quaternion (heading, radians
    /// clockwise from north).
    fn yaw_of(q: [f32; 4]) -> f64 {
        let (w, x, y, z) = (
            f64::from(q[0]),
            f64::from(q[1]),
            f64::from(q[2]),
            f64::from(q[3]),
        );
        (2.0 * (w * z + x * y)).atan2(1.0 - 2.0 * (y * y + z * z))
    }
}

impl VehicleAdapter for AviateAdapter {
    fn capabilities(&self) -> AdapterCapabilities {
        AdapterCapabilities {
            execution: ExecutionMode {
                real_time: true,
                ..ExecutionMode::default()
            },
            // Telemetry-only in this increment (ADR-0018): no control
            // scopes are advertised, so leases have nothing to grant and
            // every control frame is rejected below.
            vehicles: vec![VehicleDescriptor {
                id: self.vehicle,
                scopes: vec![],
                link_loss_actions: vec![],
            }],
            adapter_version: env!("CARGO_PKG_VERSION").to_owned(),
        }
    }

    fn apply_control(&mut self, frame: &ScopedControlFrame) -> ApplyOutcome {
        let disposition = if frame.vehicle == self.vehicle {
            // Command uplink goes through Aviate's security gateway when
            // it lands (ADR-0018); until then the boundary is closed.
            Disposition::Rejected(RejectReason::UnknownScope)
        } else {
            Disposition::Rejected(RejectReason::UnknownVehicle)
        };
        ApplyOutcome {
            tick: self.step(StepBudget { ticks: 0 }).now,
            disposition,
        }
    }

    fn sample_telemetry(&mut self) -> TelemetryBatch {
        let Ok(latest) = self.state.lock() else {
            return TelemetryBatch::default();
        };
        let Some(kin) = latest.kinematics else {
            return TelemetryBatch::default();
        };
        if kin.received_at.elapsed() > WITHHOLD_AFTER {
            return TelemetryBatch::default();
        }
        let attitude = latest
            .attitude
            .filter(|att| att.received_at.elapsed() <= WITHHOLD_AFTER);

        let heading = attitude.map_or(0.0, |att| Self::yaw_of(att.quat_wxyz));
        let speed = f64::from(
            (kin.vel_ned_mps[0] * kin.vel_ned_mps[0] + kin.vel_ned_mps[1] * kin.vel_ned_mps[1])
                .sqrt(),
        );
        let avionics = attitude.map(|att| AvionicsSample {
            quat_wxyz: att.quat_wxyz,
            rates_rps: att.rates_rps,
            pos_ned_m: kin.pos_ned_m,
            vel_ned_mps: kin.vel_ned_mps,
            // Aviate's wire subset does not carry its StateValidFlags /
            // EstimateQuality yet (ADR-0018 names the gap); freshness is
            // the only validity dimension this link can honestly claim.
            valid_flags: 0b1111,
            quality: 0,
        });
        TelemetryBatch {
            samples: vec![TelemetrySample {
                vehicle: self.vehicle,
                tick: SimTick::new(u64::from(kin.time_boot_ms).wrapping_mul(1_000_000)),
                pose: Pose2d {
                    x: f64::from(kin.pos_ned_m[0]),
                    y: f64::from(kin.pos_ned_m[1]),
                    heading,
                },
                speed,
                avionics,
            }],
        }
    }

    fn video_sources(&self) -> Vec<VideoSource> {
        vec![]
    }

    fn set_link_loss_policy(&mut self, vehicle: VehicleId, policy: Option<LinkLossPolicy>) {
        if vehicle == self.vehicle {
            // Telemetry-only: nothing to actuate; recorded for
            // capability-conformance visibility.
            self.link_loss_policy = policy;
        }
    }

    fn step(&mut self, _budget: StepBudget) -> StepOutcome {
        let tick = self
            .state
            .lock()
            .ok()
            .and_then(|latest| latest.kinematics)
            .map_or(0, |kin| u64::from(kin.time_boot_ms).wrapping_mul(1_000_000));
        StepOutcome {
            advanced: 0,
            now: SimTick::new(tick),
        }
    }
}

#[cfg(test)]
mod tests;
