//! The `VehicleAdapter` implementation for the single-skiff reference
//! vehicle (ADR-0008).

use pilotage_adapter_api::{
    AdapterCapabilities, ApplyOutcome, Disposition, ExecutionMode, IntentCapability,
    LegacyAxisRoute, LegacyCommandMap, LinkLossEnactError, LinkLossPolicy, Pose2d, RejectReason,
    ScopeDescriptor, StepBudget, StepOutcome, TelemetryBatch, TelemetrySample, VehicleAdapter,
    VehicleDescriptor, VideoSource,
};
use pilotage_protocol::{
    ControlIntent, IntentFamily, LogicalAxisId, ReferenceFrame, ScopeId, ScopedControlFrame,
    VehicleId,
};
use pilotage_timing::SimTick;
use serde::{Deserialize, Serialize};

use crate::controls::{
    ControlState, MAX_SURGE_MPS, MAX_TURN_RPS, MOTION_SCOPE, STEERING_AXIS, THROTTLE_AXIS,
};
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
                    authority_group: None,
                    scope: ScopeId::new(MOTION_SCOPE),
                    axes: vec![
                        LogicalAxisId::new(THROTTLE_AXIS),
                        LogicalAxisId::new(STEERING_AXIS),
                    ],
                    // The skiff's dynamics consume normalized surge/turn, so
                    // the advertised envelope IS the normalized bound: a
                    // full-scale typed command maps 1:1 onto full stick.
                    intents: vec![IntentCapability {
                        max_yaw_rate: 0.0,
                        family: IntentFamily::Velocity,
                        frames: vec![ReferenceFrame::BodyFrd],
                        max_linear: MAX_SURGE_MPS,
                        max_vertical: 0.0,
                        max_angular: MAX_TURN_RPS,
                    }],
                    actions: vec![],
                    legacy: Some(LegacyCommandMap::Velocity {
                        vx: Some(LegacyAxisRoute {
                            axis: THROTTLE_AXIS,
                            sign: 1.0,
                        }),
                        vy: None,
                        vz: None,
                        yaw_rate: Some(LegacyAxisRoute {
                            axis: STEERING_AXIS,
                            sign: 1.0,
                        }),
                        arm_button: None,
                        disarm_button: None,
                    }),
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
            return ApplyOutcome::new(self.tick, Disposition::Rejected(reason));
        }
        // The link-loss latch: `tick_link_loss` already forces the dynamics
        // to the policy state, but accepting frames while engaged would
        // store deflected controls that spring back the instant the policy
        // clears. Suppress them typed instead.
        if self.controls.policy_engaged() {
            return ApplyOutcome::new(
                self.tick,
                Disposition::Rejected(RejectReason::LinkLossEngaged),
            );
        }
        // Typed-only consumption: the session host translates any legacy
        // payload at its compatibility boundary, so a payload reaching this
        // adapter is a contract violation, not a fallback to honor.
        let Some(ControlIntent::Velocity(velocity)) = frame.intent else {
            return ApplyOutcome::new(
                self.tick,
                Disposition::Rejected(RejectReason::Other(
                    "the skiff consumes typed velocity intents only".to_owned(),
                )),
            );
        };
        // The typed command is metres-per-second inside the advertised
        // envelope; the dynamics consume normalized surge/turn, so divide by
        // the same limits the capability advertises. Out-of-envelope values
        // (a session bug or a direct driver) clamp rather than amplify, and
        // any non-finite would already have failed wire decoding — but the
        // dynamics' finiteness guarantee must not depend on the caller, so
        // clamp defensively.
        let (throttle, throttle_clamped) = clamp_axis(f64::from(velocity.vx / MAX_SURGE_MPS));
        let (steering, steering_clamped) = clamp_axis(f64::from(velocity.yaw_rate / MAX_TURN_RPS));
        // A lateral or vertical component is physically meaningless for a
        // diff-drive skiff: constrained, not rejected, so a generic velocity
        // sender still drives the components that exist.
        let unsupported_component = velocity.vy != 0.0 || velocity.vz != 0.0;
        self.controls.apply(throttle, steering);
        ApplyOutcome::new(
            self.tick,
            if throttle_clamped || steering_clamped || unsupported_component {
                Disposition::Constrained
            } else {
                Disposition::Accepted
            },
        )
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
    use super::{MAX_SURGE_MPS, MAX_TURN_RPS, ReferenceAdapter, StepBudget, VehicleAdapter};
    use pilotage_adapter_api::{Disposition, RejectReason};
    use pilotage_protocol::{
        ControlIntent, ControlPayload, Generation, LogicalAxisId, ReferenceFrame, ScopeId,
        ScopedControlFrame, SequenceNum, SessionId, VehicleId, VelocityIntent,
    };
    use pilotage_timing::MonoTimestamp;

    fn frame(scope: &str, intent: Option<ControlIntent>, vehicle: VehicleId) -> ScopedControlFrame {
        ScopedControlFrame {
            action_ids: vec![],
            session: SessionId::new(1),
            vehicle,
            scope: ScopeId::new(scope),
            generation: Generation::new(1),
            sequence: SequenceNum::new(1),
            sampled_at: MonoTimestamp::from_nanos(0),
            profile_revision: 1,
            activation_revision: 0,
            payload: ControlPayload::default(),
            intent,
            actions: vec![],
        }
    }

    fn velocity(vx: f32, yaw_rate: f32) -> Option<ControlIntent> {
        Some(ControlIntent::Velocity(VelocityIntent {
            frame: ReferenceFrame::BodyFrd,
            vx,
            vy: 0.0,
            vz: 0.0,
            yaw_rate,
        }))
    }

    #[test]
    fn unknown_scope_is_rejected() {
        let vehicle = VehicleId::new(1);
        let mut adapter = ReferenceAdapter::from_seed(vehicle, 1);
        let outcome = adapter.apply_control(&frame("vehicle.camera", velocity(0.0, 0.0), vehicle));
        assert_eq!(
            outcome.disposition,
            Disposition::Rejected(RejectReason::UnknownScope)
        );
    }

    /// The typed-only contract: a legacy numeric payload reaching the adapter
    /// is a session-boundary violation, rejected rather than interpreted.
    #[test]
    fn a_legacy_payload_frame_is_rejected() {
        let vehicle = VehicleId::new(1);
        let mut adapter = ReferenceAdapter::from_seed(vehicle, 1);
        let mut legacy = frame("vehicle.motion", None, vehicle);
        legacy.payload = ControlPayload {
            axes: vec![(LogicalAxisId::new(2), 1.0)],
            edges: vec![],
        };
        let outcome = adapter.apply_control(&legacy);
        assert!(matches!(
            outcome.disposition,
            Disposition::Rejected(RejectReason::Other(_))
        ));
    }

    #[test]
    fn a_typed_velocity_drives_the_dynamics() {
        let vehicle = VehicleId::new(1);
        let mut adapter = ReferenceAdapter::from_seed(vehicle, 1);
        let outcome = adapter.apply_control(&frame(
            "vehicle.motion",
            velocity(MAX_SURGE_MPS, 0.0),
            vehicle,
        ));
        assert_eq!(outcome.disposition, Disposition::Accepted);
        let step_outcome = adapter.step(StepBudget { ticks: 1 });
        assert_eq!(step_outcome.advanced, 1);
        let telemetry = adapter.sample_telemetry();
        assert!(telemetry.samples[0].speed.expect("speed") > 0.0);
    }

    #[test]
    fn an_out_of_envelope_velocity_is_clamped_and_reported_constrained() {
        let vehicle = VehicleId::new(1);
        let mut adapter = ReferenceAdapter::from_seed(vehicle, 1);
        let outcome = adapter.apply_control(&frame(
            "vehicle.motion",
            velocity(1.0e30, MAX_TURN_RPS),
            vehicle,
        ));
        assert_eq!(outcome.disposition, Disposition::Constrained);
        // Even after many ticks the trajectory stays finite: the clamp keeps
        // the surge command at full scale rather than the enormous raw value.
        adapter.step(StepBudget { ticks: 64 });
        let telemetry = adapter.sample_telemetry();
        let sample = &telemetry.samples[0];
        let pose = sample.pose.expect("pose");
        assert!(sample.speed.expect("speed").is_finite());
        assert!(pose.x.is_finite());
        assert!(pose.y.is_finite());
        assert!(pose.heading.is_finite());
    }

    /// Wire decoding rejects non-finite intents, but the dynamics' finiteness
    /// guarantee must not depend on the caller being the session host.
    #[test]
    fn a_nan_velocity_is_neutralized_and_snapshot_round_trips() {
        let vehicle = VehicleId::new(1);
        let mut adapter = ReferenceAdapter::from_seed(vehicle, 1);
        let outcome =
            adapter.apply_control(&frame("vehicle.motion", velocity(f32::NAN, 0.0), vehicle));
        assert_eq!(outcome.disposition, Disposition::Constrained);
        adapter.step(StepBudget { ticks: 4 });
        let telemetry = adapter.sample_telemetry();
        assert!(telemetry.samples[0].speed.expect("speed").is_finite());
        // A finite snapshot restores cleanly, preserving the replay contract.
        let json = adapter.snapshot().expect("snapshot encodes");
        let restored = ReferenceAdapter::restore(&json).expect("snapshot restores");
        assert_eq!(restored, adapter);
    }

    /// A lateral component a diff-drive cannot execute is constrained (the
    /// executable components still apply), never silently accepted as-is.
    #[test]
    fn an_inexecutable_lateral_component_reports_constrained() {
        let vehicle = VehicleId::new(1);
        let mut adapter = ReferenceAdapter::from_seed(vehicle, 1);
        let mut intent = velocity(0.5, 0.0);
        if let Some(ControlIntent::Velocity(ref mut v)) = intent {
            v.vy = 0.5;
        }
        let outcome = adapter.apply_control(&frame("vehicle.motion", intent, vehicle));
        assert_eq!(outcome.disposition, Disposition::Constrained);
    }
}
