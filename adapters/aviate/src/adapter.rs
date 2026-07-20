//! `VehicleAdapter` implementation over the Aviate vehicle's role-bound
//! links (ADR-0019, LINK-04): the MAVLink link carries the FC
//! operational estimate, the co-located shm block carries simulation
//! truth, and the uplink socket carries FC-owned state reports.

use std::time::Duration;

use pilotage_adapter_api::{
    AdapterCapabilities, ApplyOutcome, Disposition, ExecutionMode, LinkLossPolicy, RejectReason,
    ScopeDescriptor, SourceIncarnation, StepBudget, StepOutcome, TelemetryBatch, TelemetrySample,
    VehicleAdapter, VehicleDescriptor, VideoSource,
};
use pilotage_protocol::{
    ButtonEdge, LogicalAxisId, LogicalButtonId, ScopeId, ScopedControlFrame, VehicleId,
};
use pilotage_timing::SimTick;

#[cfg(test)]
use std::sync::{Arc, Mutex};

#[cfg(test)]
use pilotage_mavlink::link::LinkState;

use crate::error::AviateAdapterError;
use crate::uplink::FlightUplink;

mod camera;
mod control;
mod sampling;
mod shm_sampling;
mod sources;
mod startup;
use control::{normalized_flight_sticks, rejected_control};
use sampling::mavlink_batch;
use shm_sampling::ShmSource;
use sources::{ArmReport, EstimateSource, fc_state_sample};

/// The control scope exposes four canonical flight axes as DJI-style
/// velocity demands.
pub const FLIGHT_SCOPE: &str = "vehicle.motion";
/// Canonical `roll` axis (0): lateral velocity, + = right.
pub const ROLL_AXIS: u16 = 0;
/// Canonical `pitch` axis (1): forward velocity, + = forward.
pub const PITCH_AXIS: u16 = 1;
/// Canonical `throttle` axis (2): climb rate, + = climb.
pub const THROTTLE_AXIS: u16 = 2;
/// Canonical `yaw` axis (3): yaw rate, + = clockwise.
pub const YAW_AXIS: u16 = 3;
/// Logical button whose press arms the vehicle.
pub const ARM_BUTTON: u16 = 0;
/// Logical button whose press disarms the vehicle.
pub const DISARM_BUTTON: u16 = 1;
/// Logical button whose press resets the simulation (runs the reset
/// script; SITL-only convenience).
pub const RESET_BUTTON: u16 = 2;
/// Logical button toggling between camera mode (velocity sticks,
/// brake-to-hold) and FPV mode (attitude sticks, direct thrust).
pub const FPV_TOGGLE_BUTTON: u16 = 3;

/// Data older than this is withheld from telemetry entirely, so
/// downstream freshness models see the group's age grow instead of a
/// frozen value replaying forever (the same withholding discipline as
/// the Gazebo adapter's dead-reader path).
const WITHHOLD_AFTER: Duration = Duration::from_secs(3);

/// Which session profile the adapter runs (LINK-04). A profile binds
/// source ROLES — the MAVLink link carries the FC operational estimate,
/// the shm block carries simulation truth — and transports are never
/// alternatives for one another.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AviateProfile {
    /// Physical vehicle: FC estimate + FC state. A truth source must not
    /// exist and is never synthesized.
    Physical,
    /// Simulation: FC estimate + FC state, plus the simulation-truth
    /// oracle while the co-located shm block is attachable.
    #[default]
    Simulation,
    /// Oracle-only diagnostics: the truth stream alone. No uplink is
    /// bound and no motion-control scope is advertised — operational
    /// control is structurally absent, not merely rejected.
    OracleOnly,
}

/// Telemetry-only adapter for the Aviate flight controller (ADR-0018).
///
/// Real-time (ADR-0013): the FC/simulation advances on its own clock;
/// `step` reports the latest observed vehicle time as the simulation
/// tick.
#[derive(Debug)]
pub struct AviateAdapter {
    vehicle: VehicleId,
    // Source roles are structural (LINK-04): the MAVLink link only ever
    // produces the FC operational estimate and the shm link only ever
    // produces the simulation-truth oracle. Neither substitutes for the
    // other: a missing estimate rejects state-dependent control instead
    // of borrowing truth.
    estimate: Option<EstimateSource>,
    truth: Option<Box<ShmSource>>,
    uplink: Option<FlightUplink>,
    // Pilotage's Gazebo sidecar bridges the flight world's camera topics;
    // the adapter remains usable without video when the sidecar cannot spawn.
    frames: Option<tokio::sync::mpsc::Receiver<pilotage_adapter_gazebo::RawVideoFrame>>,
    _camera_bridge: Option<pilotage_adapter_gazebo::BridgeClient>,
    _frame_forwarder: Option<tokio::task::JoinHandle<()>>,
    // Latest FC arm report from uplink heartbeats, with its receive
    // metadata; `None` until the FC has reported at least once.
    arm: Option<ArmReport>,
    // Identity under which arm reports are stamped.
    arm_incarnation: SourceIncarnation,
    // Zero point of the host-monotonic acquisition clock.
    started_at: std::time::Instant,
    last_reset: Option<std::time::Instant>,
    link_loss_policy: Option<LinkLossPolicy>,
    // Commanded-reset latch: engaged when a sim reset is requested,
    // cleared only by a fresh estimate source epoch plus demonstrated
    // neutral input (control::ResetLatch). While engaged, everything
    // except disarm is rejected.
    reset_latch: Option<control::ResetLatch>,
    // Reset script spawns recorded instead of executed, so tests can
    // press the reset button without running the real script (which
    // kills any live SITL FC on the machine).
    #[cfg(test)]
    reset_spawns: u32,
    // FPV mode latch (FPV_TOGGLE_BUTTON): attitude sticks + direct
    // thrust instead of velocity sticks + brake-to-hold.
    fpv_mode: bool,
}

impl AviateAdapter {
    /// Takes the raw-frame receiver for the host media task, if cameras
    /// are up and it has not been taken.
    pub fn subscribe_frames(
        &mut self,
    ) -> Option<tokio::sync::mpsc::Receiver<pilotage_adapter_gazebo::RawVideoFrame>> {
        self.frames.take()
    }

    /// The typed fault that has fail-closed the simulation-truth source,
    /// if any. A faulted source publishes no telemetry and does not
    /// re-attach. Only the shared-memory truth source carries a
    /// fail-closed fault state; the MAVLink estimate link reports `None`
    /// here.
    pub fn shm_fault(&self) -> Option<&AviateAdapterError> {
        self.truth.as_ref().and_then(|source| source.fault())
    }

    /// Wires an adapter around a caller-supplied state cache, for tests.
    #[cfg(test)]
    pub(crate) fn from_state(vehicle: VehicleId, state: Arc<Mutex<LinkState>>) -> Self {
        Self {
            vehicle,
            estimate: Some(EstimateSource { state, _link: None }),
            truth: None,
            uplink: None,
            frames: None,
            _camera_bridge: None,
            _frame_forwarder: None,
            arm: None,
            arm_incarnation: SourceIncarnation::new([0; 16]),
            started_at: std::time::Instant::now(),
            last_reset: None,
            reset_latch: None,
            reset_spawns: 0,
            fpv_mode: false,
            link_loss_policy: None,
        }
    }

    /// Expires the bound uplink's post-arm quiet window, so tests step
    /// past it deterministically instead of sleeping wall-clock time.
    #[cfg(test)]
    pub(crate) fn expire_uplink_quiet_for_test(&mut self) {
        if let Some(uplink) = self.uplink.as_mut() {
            uplink.expire_quiet_for_test();
        }
    }

    /// Whether the bound uplink currently holds a captured position-hold
    /// point, for tests of the link-loss hold-invalidation contract.
    #[cfg(test)]
    pub(crate) fn uplink_hold_captured(&self) -> bool {
        self.uplink
            .as_ref()
            .is_some_and(crate::uplink::FlightUplink::hold_captured)
    }

    /// Installs a test uplink, for tests.
    #[cfg(test)]
    pub(crate) fn with_uplink(mut self, uplink: FlightUplink) -> Self {
        self.uplink = Some(uplink);
        self
    }

    /// The bound uplink, for tests that drive its manual clock.
    #[cfg(test)]
    pub(crate) fn uplink_mut(&mut self) -> Option<&mut FlightUplink> {
        self.uplink.as_mut()
    }

    fn validate_flight_frame(&self, frame: &ScopedControlFrame) -> Result<(), RejectReason> {
        if frame.vehicle != self.vehicle {
            return Err(RejectReason::UnknownVehicle);
        }
        if frame.scope.as_str() != FLIGHT_SCOPE {
            return Err(RejectReason::UnknownScope);
        }
        let known = [
            LogicalAxisId::new(ROLL_AXIS),
            LogicalAxisId::new(PITCH_AXIS),
            LogicalAxisId::new(THROTTLE_AXIS),
            LogicalAxisId::new(YAW_AXIS),
        ];
        for (axis, _) in &frame.payload.axes {
            if !known.contains(axis) {
                return Err(RejectReason::UnknownAxis);
            }
        }
        Ok(())
    }
}

impl VehicleAdapter for AviateAdapter {
    fn capabilities(&self) -> AdapterCapabilities {
        AdapterCapabilities {
            execution: ExecutionMode {
                real_time: true,
                render_capable: self._camera_bridge.is_some(),
                ..ExecutionMode::default()
            },
            // Without a working velocity-control uplink, the adapter stays
            // telemetry-only as required by ADR-0018.
            vehicles: vec![VehicleDescriptor {
                id: self.vehicle,
                scopes: if self.uplink.is_some() {
                    vec![ScopeDescriptor {
                        scope: ScopeId::new(FLIGHT_SCOPE),
                        axes: vec![
                            LogicalAxisId::new(ROLL_AXIS),
                            LogicalAxisId::new(PITCH_AXIS),
                            LogicalAxisId::new(THROTTLE_AXIS),
                            LogicalAxisId::new(YAW_AXIS),
                        ],
                    }]
                } else {
                    vec![]
                },
                link_loss_actions: if self.uplink.is_some() {
                    vec![LinkLossPolicy::Neutralize]
                } else {
                    vec![]
                },
            }],
            adapter_version: env!("CARGO_PKG_VERSION").to_owned(),
        }
    }

    fn apply_control(&mut self, frame: &ScopedControlFrame) -> ApplyOutcome {
        let tick = self.step(StepBudget { ticks: 0 }).now;
        if let Some(outcome) = self.gated_flight_outcome(frame, tick) {
            return outcome;
        }
        let Some((current_yaw, current_pos, current_vel)) = self.current_pose() else {
            return rejected_control(tick, RejectReason::MeasurementUnavailable);
        };
        let Some(uplink) = self.uplink.as_mut() else {
            return rejected_control(tick, RejectReason::UnknownScope);
        };

        for (button, edge) in &frame.payload.edges {
            if *edge != ButtonEdge::Pressed {
                continue;
            }
            if *button == LogicalButtonId::new(ARM_BUTTON) {
                uplink.send_arm(current_yaw);
            } else if *button == LogicalButtonId::new(FPV_TOGGLE_BUTTON) {
                self.fpv_mode = !self.fpv_mode;
                tracing::info!(fpv = self.fpv_mode, "flight mode toggled");
            }
        }

        let (sticks, transformed) = normalized_flight_sticks(frame);
        if self.fpv_mode {
            uplink.send_fpv_frame(
                sticks[usize::from(ROLL_AXIS)],
                sticks[usize::from(PITCH_AXIS)],
                sticks[usize::from(THROTTLE_AXIS)],
                sticks[usize::from(YAW_AXIS)],
            );
        } else {
            uplink.send_stick_frame(
                sticks[usize::from(ROLL_AXIS)],
                sticks[usize::from(PITCH_AXIS)],
                sticks[usize::from(THROTTLE_AXIS)],
                sticks[usize::from(YAW_AXIS)],
                current_yaw,
                current_pos,
                current_vel,
            );
        }
        ApplyOutcome {
            tick,
            disposition: if transformed {
                Disposition::Transformed
            } else {
                Disposition::Accepted
            },
        }
    }

    fn sample_telemetry(&mut self) -> TelemetryBatch {
        if let Some(uplink) = self.uplink.as_mut()
            && let Some(armed) = uplink.poll_fc()
        {
            let (system_id, component_id) = uplink.expected_source();
            let sequence = self.arm.map_or(0, |report| report.sequence.wrapping_add(1));
            self.arm = Some(ArmReport {
                armed,
                system_id,
                component_id,
                sequence,
                acquired_at: std::time::Instant::now(),
            });
        }
        let fc_state = fc_state_sample(self.arm, self.arm_incarnation, self.started_at);
        let truth = self.truth.as_mut().and_then(|source| source.truth_sample());
        let mut batch = match &self.estimate {
            Some(source) => mavlink_batch(self.vehicle, &source.state),
            None => TelemetryBatch::default(),
        };
        if let Some(sample) = batch.samples.first_mut() {
            sample.sim_truth = truth;
            sample.fc_state = fc_state;
            return batch;
        }
        // No estimate sample this tick: the truth oracle and the FC's
        // stamped state report still publish under their own identities —
        // with the panels' avionics estimate honestly absent, never
        // synthesized from truth. A healthy FC heartbeat alone is a
        // publishable observation; it must not vanish because no other
        // source produced a sample.
        if truth.is_some() || fc_state.is_some() {
            return TelemetryBatch {
                samples: vec![TelemetrySample {
                    vehicle: self.vehicle,
                    // Without a simulation clock the tick has no source;
                    // FC-state freshness reasoning uses its stamp, never
                    // this transport tick.
                    tick: SimTick::new(
                        truth
                            .as_ref()
                            .map_or(0, |sample| sample.stamp.acquired_at_ns),
                    ),
                    pose: None,
                    speed: None,
                    avionics: None,
                    sim_truth: truth,
                    fc_state,
                    gimbal: None,
                }],
            };
        }
        batch
    }

    fn video_sources(&self) -> Vec<VideoSource> {
        if self._camera_bridge.is_none() {
            return vec![];
        }
        vec![
            VideoSource {
                id: pilotage_adapter_gazebo::FPV_SOURCE_ID.to_owned(),
                description: "onboard forward camera".to_owned(),
            },
            VideoSource {
                id: pilotage_adapter_gazebo::CHASE_SOURCE_ID.to_owned(),
                description: "chase camera".to_owned(),
            },
        ]
    }

    fn set_link_loss_policy(
        &mut self,
        vehicle: VehicleId,
        policy: Option<LinkLossPolicy>,
    ) -> Result<(), pilotage_adapter_api::LinkLossEnactError> {
        if vehicle != self.vehicle {
            return Err(pilotage_adapter_api::LinkLossEnactError::UnknownVehicle { vehicle });
        }
        // Latch first, fail after: even an unenactable engage suppresses
        // ordinary control frames. Any link-loss transition also
        // invalidates the captured position-hold context — a hold point
        // captured under the lost lease is obsolete, and letting it
        // survive would command recovery back toward it the instant
        // control resumes.
        self.link_loss_policy = policy;
        if let Some(uplink) = self.uplink.as_mut() {
            uplink.clear_hold_state();
        }
        if policy.is_some() {
            // Engaging any policy sends a zero-velocity setpoint: the FC's
            // velocity mode brakes to a hover, which is the only safe action
            // a camera drone has (`Neutralize`). Clearing (link recovery)
            // leaves the FC hovering until the operator commands again.
            let Some(uplink) = self.uplink.as_mut() else {
                return Err(pilotage_adapter_api::LinkLossEnactError::NoActuationChannel);
            };
            // Success is only claimed for a datagram the socket accepted;
            // a refused send must reach the host's fail-closed counter,
            // not vanish into a log line. The uplink counts refused sends,
            // so an increment across this send IS the refusal.
            let failures_before = uplink.send_failures();
            uplink.send_neutral();
            if uplink.send_failures() != failures_before {
                return Err(pilotage_adapter_api::LinkLossEnactError::ChannelRejected {
                    detail: "the neutral setpoint datagram was not sent".to_owned(),
                });
            }
        }
        Ok(())
    }

    fn step(&mut self, _budget: StepBudget) -> StepOutcome {
        // The simulation clock is sim infrastructure, not vehicle state:
        // when the truth oracle is bound its time drives the session
        // tick; otherwise the estimate's source time does.
        let tick = if let Some(source) = &self.truth {
            source.tick()
        } else if let Some(source) = &self.estimate {
            source
                .state
                .lock()
                .ok()
                .and_then(|latest| latest.kinematics)
                .map_or(0, |kin| u64::from(kin.time_boot_ms).wrapping_mul(1_000_000))
        } else {
            0
        };
        StepOutcome {
            advanced: 0,
            now: SimTick::new(tick),
        }
    }
}

#[cfg(test)]
mod tests;
