//! The `VehicleAdapter` implementation for the single-skiff reference
//! vehicle (ADR-0008).

use pilotage_adapter_api::{
    AdapterCapabilities, ApplyOutcome, Disposition, ExecutionMode, LinkLossEnactError,
    LinkLossPolicy, Pose2d, RejectReason, ScopeDescriptor, StepBudget, StepOutcome, TelemetryBatch,
    TelemetrySample, VehicleAdapter, VehicleDescriptor, VideoSource,
};
use pilotage_protocol::{LogicalAxisId, ScopeId, ScopedControlFrame, VehicleId};
use pilotage_timing::SimTick;
use serde::{Deserialize, Serialize};

use crate::controls::{ControlState, MOTION_SCOPE, STEERING_AXIS, THROTTLE_AXIS};
use crate::scenario::initial_state_from_seed;
use crate::skiff::SkiffState;

/// The reference adapter's adapter-version string reported in capabilities.
pub const ADAPTER_VERSION: &str = "0.1.0";

/// Full serializable state of the reference adapter: everything needed to
/// resume an identical trajectory from a snapshot (ADR-0008, ADR-0012).
///
/// `vehicle` and `tick` are stored as raw `u64` rather than the
/// `pilotage-protocol`/`pilotage-timing` newtypes: those newtypes
/// deliberately do not implement `Serialize`/`Deserialize` themselves, since
/// wire encoding for them belongs to the protocol crate's schema-generated
/// types (ADR-0002), not to this adapter's local snapshot format.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReferenceAdapterSnapshot {
    /// The single vehicle this adapter drives, as a raw id.
    pub vehicle: u64,
    /// Current simulation tick, as a raw count.
    pub tick: u64,
    /// Current dynamic state.
    pub skiff: SkiffState,
    /// Current latest-valid-value controls and link-loss bookkeeping.
    pub controls: ControlState,
}

/// The deterministic, headless, single-vehicle reference adapter (ADR-0008):
/// a v1 conformance anchor independent of any graphical or commercial
/// engine.
#[derive(Debug, Clone, PartialEq)]
pub struct ReferenceAdapter {
    vehicle: VehicleId,
    tick: SimTick,
    skiff: SkiffState,
    controls: ControlState,
}

impl ReferenceAdapter {
    /// Constructs a fresh adapter for `vehicle`, with its initial state
    /// derived from `seed` (ADR-0013's deterministic-reset requirement).
    #[must_use]
    pub fn from_seed(vehicle: VehicleId, seed: u64) -> Self {
        Self {
            vehicle,
            tick: SimTick::new(0),
            skiff: initial_state_from_seed(seed),
            controls: ControlState::default(),
        }
    }

    /// Serializes the adapter's full state to JSON.
    ///
    /// # Errors
    ///
    /// Returns `serde_json::Error` if the snapshot cannot be encoded, which
    /// should not occur for this crate's own plain-data types.
    pub fn snapshot(&self) -> Result<String, serde_json::Error> {
        let snapshot = ReferenceAdapterSnapshot {
            vehicle: self.vehicle.as_u64(),
            tick: self.tick.as_u64(),
            skiff: self.skiff,
            controls: self.controls,
        };
        serde_json::to_string(&snapshot)
    }

    /// Restores an adapter from a JSON snapshot produced by
    /// [`Self::snapshot`].
    ///
    /// # Errors
    ///
    /// Returns `serde_json::Error` if `json` is not a valid encoding of
    /// [`ReferenceAdapterSnapshot`].
    pub fn restore(json: &str) -> Result<Self, serde_json::Error> {
        let snapshot: ReferenceAdapterSnapshot = serde_json::from_str(json)?;
        Ok(Self {
            vehicle: VehicleId::new(snapshot.vehicle),
            tick: SimTick::new(snapshot.tick),
            skiff: snapshot.skiff,
            controls: snapshot.controls,
        })
    }

    fn validate_frame(&self, frame: &ScopedControlFrame) -> Result<(), RejectReason> {
        if frame.vehicle != self.vehicle {
            return Err(RejectReason::UnknownVehicle);
        }
        if frame.scope.as_str() != MOTION_SCOPE {
            return Err(RejectReason::UnknownScope);
        }
        let known_axes = [
            LogicalAxisId::new(THROTTLE_AXIS),
            LogicalAxisId::new(STEERING_AXIS),
        ];
        for (axis, _) in &frame.payload.axes {
            if !known_axes.contains(axis) {
                return Err(RejectReason::UnknownAxis);
            }
        }
        Ok(())
    }
}

/// Coerces a raw axis value into the documented `[-1.0, 1.0]` range,
/// returning the clamped value and whether it differed from the input.
///
/// NaN maps to `0.0` (neutral) since it has no meaningful sign; infinities
/// map to the corresponding range bound. This guarantees the value fed to
/// [`SkiffState::step`] is finite and in range, so telemetry never diverges to
/// a non-finite `f64` that would serialize to JSON `null` and break snapshot
/// restore (ADR-0008, ADR-0012).
fn clamp_axis(value: f64) -> (f64, bool) {
    let clamped = if value.is_nan() {
        0.0
    } else {
        value.clamp(-1.0, 1.0)
    };
    (clamped, clamped != value)
}

impl VehicleAdapter for ReferenceAdapter {
    fn capabilities(&self) -> AdapterCapabilities {
        AdapterCapabilities {
            execution: ExecutionMode {
                real_time: false,
                stepped: true,
                accelerated: true,
                deterministic: true,
                render_capable: false,
                physically_embodied: false,
            },
            vehicles: vec![VehicleDescriptor {
                id: self.vehicle,
                scopes: vec![ScopeDescriptor {
                    scope: ScopeId::new(MOTION_SCOPE),
                    axes: vec![
                        LogicalAxisId::new(THROTTLE_AXIS),
                        LogicalAxisId::new(STEERING_AXIS),
                    ],
                }],
                link_loss_actions: vec![
                    LinkLossPolicy::Neutralize,
                    LinkLossPolicy::HoldBrief { ticks: 0 },
                ],
            }],
            adapter_version: ADAPTER_VERSION.to_owned(),
        }
    }

    fn apply_control(&mut self, frame: &ScopedControlFrame) -> ApplyOutcome {
        if let Err(reason) = self.validate_frame(frame) {
            return ApplyOutcome {
                tick: self.tick,
                disposition: Disposition::Rejected(reason),
            };
        }
        // The link-loss latch: `tick_link_loss` already forces the dynamics
        // to the policy state, but accepting frames while engaged would
        // store deflected controls that spring back the instant the policy
        // clears. Suppress them typed instead.
        if self.controls.policy_engaged() {
            return ApplyOutcome {
                tick: self.tick,
                disposition: Disposition::Rejected(RejectReason::LinkLossEngaged),
            };
        }
        let mut throttle = self.controls.throttle;
        let mut steering = self.controls.steering;
        let mut transformed = false;
        for (axis, value) in &frame.payload.axes {
            // Wire decoding only guarantees finiteness, not range: a finite
            // f32 like 1e30 would drive speed to +inf within a few ticks, and
            // any non-finite value poisons pos/heading/speed permanently and
            // then serializes to JSON null, breaking the snapshot/replay
            // contract (ADR-0008, ADR-0012). SkiffState::step assumes its
            // inputs are already in [-1, 1], so clamp here.
            let (clamped, changed) = clamp_axis(f64::from(*value));
            transformed |= changed;
            if *axis == LogicalAxisId::new(THROTTLE_AXIS) {
                throttle = clamped;
            } else if *axis == LogicalAxisId::new(STEERING_AXIS) {
                steering = clamped;
            }
        }
        self.controls.apply(throttle, steering);
        ApplyOutcome {
            tick: self.tick,
            disposition: if transformed {
                Disposition::Transformed
            } else {
                Disposition::Accepted
            },
        }
    }

    fn sample_telemetry(&mut self) -> TelemetryBatch {
        TelemetryBatch {
            samples: vec![TelemetrySample {
                vehicle: self.vehicle,
                tick: self.tick,
                pose: Some(Pose2d {
                    x: self.skiff.pos[0],
                    y: self.skiff.pos[1],
                    heading: self.skiff.heading,
                }),
                speed: Some(self.skiff.speed),
                avionics: None,
                sim_truth: None,
                fc_state: None,
                gimbal: None,
            }],
        }
    }

    fn video_sources(&self) -> Vec<VideoSource> {
        Vec::new()
    }

    fn set_link_loss_policy(
        &mut self,
        vehicle: VehicleId,
        scope: &ScopeId,
        policy: Option<LinkLossPolicy>,
    ) -> Result<(), LinkLossEnactError> {
        if vehicle != self.vehicle {
            return Err(LinkLossEnactError::UnknownVehicle { vehicle });
        }
        // The skiff exposes only the motion scope, so only a motion-scope policy
        // touches its actuation; any other scope's link-loss is a no-op here.
        if scope.as_str() == MOTION_SCOPE {
            self.controls.set_policy(policy);
        }
        Ok(())
    }

    fn step(&mut self, budget: StepBudget) -> StepOutcome {
        for _ in 0..budget.ticks {
            let (throttle, steering) = self.controls.tick_link_loss();
            self.skiff = self.skiff.step(throttle, steering);
            self.tick = self.tick.next();
        }
        StepOutcome {
            advanced: budget.ticks,
            now: self.tick,
        }
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic)]
mod tests {
    use super::{ReferenceAdapter, StepBudget, VehicleAdapter};
    use pilotage_adapter_api::{Disposition, RejectReason};
    use pilotage_protocol::{
        ControlPayload, Generation, LogicalAxisId, ScopeId, ScopedControlFrame, SequenceNum,
        SessionId, VehicleId,
    };
    use pilotage_timing::MonoTimestamp;

    fn frame(
        scope: &str,
        axes: Vec<(LogicalAxisId, f32)>,
        vehicle: VehicleId,
    ) -> ScopedControlFrame {
        ScopedControlFrame {
            session: SessionId::new(1),
            vehicle,
            scope: ScopeId::new(scope),
            generation: Generation::new(1),
            sequence: SequenceNum::new(1),
            sampled_at: MonoTimestamp::from_nanos(0),
            profile_revision: 1,
            payload: ControlPayload {
                axes,
                edges: vec![],
            },
        }
    }

    #[test]
    fn unknown_scope_is_rejected() {
        let vehicle = VehicleId::new(1);
        let mut adapter = ReferenceAdapter::from_seed(vehicle, 1);
        let outcome = adapter.apply_control(&frame("vehicle.camera", vec![], vehicle));
        assert_eq!(
            outcome.disposition,
            Disposition::Rejected(RejectReason::UnknownScope)
        );
    }

    #[test]
    fn unknown_axis_is_rejected() {
        let vehicle = VehicleId::new(1);
        let mut adapter = ReferenceAdapter::from_seed(vehicle, 1);
        let outcome = adapter.apply_control(&frame(
            "vehicle.motion",
            vec![(LogicalAxisId::new(99), 1.0)],
            vehicle,
        ));
        assert_eq!(
            outcome.disposition,
            Disposition::Rejected(RejectReason::UnknownAxis)
        );
    }

    #[test]
    fn known_axes_are_accepted_and_drive_dynamics() {
        let vehicle = VehicleId::new(1);
        let mut adapter = ReferenceAdapter::from_seed(vehicle, 1);
        let outcome = adapter.apply_control(&frame(
            "vehicle.motion",
            vec![(LogicalAxisId::new(2), 1.0), (LogicalAxisId::new(3), 0.0)],
            vehicle,
        ));
        assert_eq!(outcome.disposition, Disposition::Accepted);
        let step_outcome = adapter.step(StepBudget { ticks: 1 });
        assert_eq!(step_outcome.advanced, 1);
        let telemetry = adapter.sample_telemetry();
        assert!(telemetry.samples[0].speed.expect("speed") > 0.0);
    }

    #[test]
    fn out_of_range_throttle_is_clamped_and_reported_transformed() {
        let vehicle = VehicleId::new(1);
        let mut adapter = ReferenceAdapter::from_seed(vehicle, 1);
        let outcome = adapter.apply_control(&frame(
            "vehicle.motion",
            vec![
                (LogicalAxisId::new(2), 1.0e30),
                (LogicalAxisId::new(3), 0.0),
            ],
            vehicle,
        ));
        assert_eq!(outcome.disposition, Disposition::Transformed);
        // Even after many ticks the trajectory stays finite: the clamp keeps
        // throttle at 1.0 rather than the enormous raw value.
        adapter.step(StepBudget { ticks: 64 });
        let telemetry = adapter.sample_telemetry();
        let sample = &telemetry.samples[0];
        let pose = sample.pose.expect("pose");
        assert!(sample.speed.expect("speed").is_finite());
        assert!(pose.x.is_finite());
        assert!(pose.y.is_finite());
        assert!(pose.heading.is_finite());
    }

    #[test]
    fn nan_throttle_is_neutralized_and_snapshot_round_trips() {
        let vehicle = VehicleId::new(1);
        let mut adapter = ReferenceAdapter::from_seed(vehicle, 1);
        let outcome = adapter.apply_control(&frame(
            "vehicle.motion",
            vec![(LogicalAxisId::new(2), f32::NAN)],
            vehicle,
        ));
        assert_eq!(outcome.disposition, Disposition::Transformed);
        adapter.step(StepBudget { ticks: 4 });
        let telemetry = adapter.sample_telemetry();
        assert!(telemetry.samples[0].speed.expect("speed").is_finite());
        // A finite snapshot restores cleanly, preserving the replay contract.
        let json = adapter.snapshot().expect("snapshot encodes");
        let restored = ReferenceAdapter::restore(&json).expect("snapshot restores");
        assert_eq!(restored, adapter);
    }
}
