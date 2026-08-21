//! The flight-scope enactment and the telemetry assembly: the two
//! largest bodies of the Aviate adapter, kept beside the adapter rather
//! than inside it so each file stays readable on its own.

use pilotage_adapter_api::{
    ApplyOutcome, Disposition, LinkLossPolicy, RejectReason, TelemetryBatch, TelemetrySample,
    VideoSource,
};
use pilotage_protocol::{ControlIntent, ScopeId, ScopedControlFrame, VehicleId};
use pilotage_timing::SimTick;

#[cfg(feature = "sim")]
use super::GIMBAL_SCOPE;
use super::sources::{ArmReport, fc_state_sample};
use super::{
    AviateAdapter, DIRECT_SCOPE, FLIGHT_SCOPE, StepBudget, StepOutcome, control::rejected_control,
    control::sticks_from_velocity, mavlink_batch, process_flight_actions,
};
use crate::uplink::FlightUplink;

fn send_motion_intent(
    uplink: &mut FlightUplink,
    frame: &ScopedControlFrame,
    current_attitude: [f32; 3],
    current_pos: [f32; 3],
    current_vel: Option<[f32; 3]>,
) -> Option<bool> {
    let envelope = uplink.envelope();
    match (frame.scope.as_str(), frame.intent) {
        (FLIGHT_SCOPE, Some(ControlIntent::Velocity(velocity))) => {
            let (sticks, constrained) = sticks_from_velocity(&velocity, envelope);
            let uplink_constrained = uplink.send_stick_frame(
                sticks[0],
                sticks[1],
                sticks[2],
                sticks[3],
                current_attitude[2],
                current_pos,
                current_vel,
                None,
            );
            Some(constrained || uplink_constrained)
        }
        (DIRECT_SCOPE, Some(ControlIntent::AttitudeThrust(attitude))) => {
            let (roll, pitch, yaw) = pilotage_adapter_api::attitude_euler(&attitude);
            let constrained = uplink.send_attitude_frame_seeded(
                roll,
                pitch,
                yaw,
                attitude.thrust,
                current_attitude,
            );
            Some(
                constrained
                    || roll.abs() > envelope.direct_tilt_rad
                    || pitch.abs() > envelope.direct_tilt_rad,
            )
        }
        _ => None,
    }
}

impl AviateAdapter {
    /// Enacts one flight-scope frame: the gate chain, then the velocity
    /// or attitude demand the scope's family carries.
    pub(super) fn apply_flight(
        &mut self,
        frame: &ScopedControlFrame,
        tick: SimTick,
    ) -> ApplyOutcome {
        if let Some(outcome) = self.gated_flight_outcome(frame, tick) {
            return outcome;
        }
        let Some(current) = self.current_pose() else {
            return rejected_control(tick, RejectReason::MeasurementUnavailable);
        };
        let Some(uplink) = self.uplink.as_mut() else {
            return rejected_control(tick, RejectReason::UnknownScope);
        };

        let action_results = process_flight_actions(
            &frame.actions,
            |yaw| {
                uplink.send_arm(yaw);
            },
            current.attitude_rad[2],
        );
        if frame.intent.is_none() {
            return ApplyOutcome {
                tick,
                disposition: Disposition::Accepted,
                action_results,
            };
        }
        let Some(constrained) = send_motion_intent(
            uplink,
            frame,
            current.attitude_rad,
            current.pos_ned_m,
            current.velocity_ned_mps,
        ) else {
            return rejected_control(
                tick,
                RejectReason::Other("intent family does not belong to this scope".to_owned()),
            );
        };
        ApplyOutcome {
            tick,
            disposition: if constrained {
                Disposition::Constrained
            } else {
                Disposition::Accepted
            },
            action_results,
        }
    }

    /// Assembles this tick's telemetry from every bound source role.
    pub(super) fn collect_telemetry(&mut self) -> TelemetryBatch {
        // One rendered view: return it to the vehicle's forward camera
        // when the operator stops aiming the payload.
        #[cfg(feature = "sim")]
        self.maintain_camera_view();
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
        let truth = self.take_truth_sample();
        let mut batch = match &self.estimate {
            Some(source) => mavlink_batch(self.vehicle, &source.state),
            None => TelemetryBatch::default(),
        };
        if let Some(sample) = batch.samples.first_mut() {
            // The shm oracle outranks the estimate-stream truth when both
            // exist, but an ABSENT shm oracle must not erase the truth the
            // estimate stream carried (the X-Plane lane's only truth path).
            if truth.is_some() {
                sample.sim_truth = truth;
            }
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

    /// The video sources this adapter exposes.
    pub(super) fn advertised_video_sources(&self) -> Vec<VideoSource> {
        if self.camera_bridge.is_none() {
            return vec![];
        }
        vec![
            VideoSource {
                id: pilotage_adapter_api::FPV_SOURCE_ID.to_owned(),
                description: "onboard forward camera".to_owned(),
            },
            VideoSource {
                id: pilotage_adapter_api::CHASE_SOURCE_ID.to_owned(),
                description: "chase camera".to_owned(),
            },
            // The producer renders ONE view, so the payload feed paints
            // only while the gimbal mode is commanded. It is advertised
            // regardless so a client can route it the moment it does;
            // an idle source ages out under the existing video-stall
            // semantics rather than being hidden and reappearing.
            VideoSource {
                id: pilotage_adapter_api::GIMBAL_SOURCE_ID.to_owned(),
                description: "gimbal payload view".to_owned(),
            },
        ]
    }

    /// Records and enacts one scope's link-loss policy.
    pub(super) fn enact_link_loss(
        &mut self,
        vehicle: VehicleId,
        scope: &ScopeId,
        policy: Option<LinkLossPolicy>,
    ) -> Result<(), pilotage_adapter_api::LinkLossEnactError> {
        if vehicle != self.vehicle {
            return Err(pilotage_adapter_api::LinkLossEnactError::UnknownVehicle { vehicle });
        }
        // Latch first, fail after: even an unenactable engage suppresses this
        // scope's control frames. The latch is per-scope so another scope's
        // link-loss never suppresses this one.
        match &policy {
            Some(policy) => {
                self.link_loss_policy.insert(scope.clone(), *policy);
            }
            None => {
                self.link_loss_policy.remove(scope);
            }
        }
        #[cfg(feature = "sim")]
        if scope.as_str() == GIMBAL_SCOPE {
            return self.enact_gimbal_link_loss(policy.is_some());
        }
        // Only the MOTION scopes actuate flight: a gimbal-scope link-loss
        // must NOT touch the FC or the motion hold context, so the
        // neutralize and the hold-invalidation below are gated on them.
        if scope.as_str() != FLIGHT_SCOPE && scope.as_str() != DIRECT_SCOPE {
            return Ok(());
        }
        // Any motion link-loss transition invalidates the captured
        // position-hold context — a hold point captured under the lost lease
        // is obsolete, and letting it survive would command recovery back
        // toward it the instant control resumes.
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

    /// The latest observed vehicle time as the session tick.
    pub(super) fn advance(&mut self, _budget: StepBudget) -> StepOutcome {
        // The simulation clock is sim infrastructure, not vehicle state:
        // when the truth oracle is bound its time drives the session
        // tick; otherwise the estimate's source time does.
        let tick = if let Some(tick) = self.truth_tick_ns() {
            tick
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
