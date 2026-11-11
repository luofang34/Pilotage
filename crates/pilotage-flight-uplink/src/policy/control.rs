//! Normal and direct control frames.

use std::time::Duration;

use pilotage_mavlink::codec::{
    AttitudeTargetKind, encode_attitude_target, encode_position_setpoint,
};

use super::feel::UplinkMode;
use super::{HeadingState, MAX_DT_S, UplinkPolicy, boot_ms};

/// One operator stick frame and the vehicle state it is shaped against.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StickFrame {
    /// Roll stick in `[-1, 1]`: a body-right velocity demand.
    pub roll: f32,
    /// Pitch stick in `[-1, 1]`: a body-forward velocity demand.
    pub pitch: f32,
    /// Throttle stick in `[-1, 1]`: a climb-rate demand.
    pub throttle: f32,
    /// Yaw stick in `[-1, 1]`: a yaw-rate demand.
    pub yaw: f32,
    /// The measured heading, radians.
    pub current_yaw_rad: f32,
    /// The measured position in the local NED frame, metres.
    pub current_pos_ned_m: [f32; 3],
    /// The measured NED velocity, if known, m/s.
    pub current_vel_ned_mps: Option<[f32; 3]>,
    /// The measured NED acceleration, if known, m/s².
    pub current_accel_ned_mps2: Option<[f32; 3]>,
}

/// What one stick or attitude frame produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StickOutcome {
    /// The frame to send, if the policy sends one.
    pub frame: Option<Vec<u8>>,
    /// Whether the policy changed or refused a part of the request.
    pub constrained: bool,
}

/// What one direct attitude frame produced.
pub type AttitudeOutcome = StickOutcome;

/// The result of preparing a control-feel activation.
#[derive(Debug, Clone, PartialEq)]
pub enum ActivationStep {
    /// The command sequence is in its quiet interval: try again later.
    NotReady,
    /// The vehicle is not airborne: the activation can commit now.
    Ready,
    /// Send this neutral setpoint. When it does not arrive, put back
    /// `prior` with [`UplinkPolicy::restore_heading`].
    Send {
        /// The neutral setpoint frame.
        frame: Vec<u8>,
        /// The heading state before the activation.
        prior: HeadingState,
    },
}

/// One exact direct attitude frame.
#[derive(Debug, Clone, PartialEq)]
pub struct DirectFrame {
    /// The MAVLink frame sequence.
    pub sequence: u8,
    /// The boot time in milliseconds.
    pub time_boot_ms: u32,
    /// The encoded frame.
    pub frame: Vec<u8>,
}

/// Why the policy sends no exact direct frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DirectRefusal {
    /// The command link is inside the post-arm quiet interval.
    QuietInterval,
    /// The setpoint stream is closed until the vehicle is airborne.
    StreamClosed,
}

impl UplinkPolicy {
    /// Prepares one neutral control-feel activation.
    ///
    /// # Errors
    ///
    /// The measured heading is not finite.
    pub fn prepare_control_feel_activation(
        &mut self,
        now: Duration,
        current_yaw_rad: f32,
    ) -> Result<ActivationStep, &'static str> {
        if !current_yaw_rad.is_finite() {
            return Err("the measured heading is not finite");
        }
        if self.in_quiet_interval(now) {
            return Ok(ActivationStep::NotReady);
        }
        let prior = self.heading;
        self.heading = HeadingState {
            heading_rad: wrap_pi(current_yaw_rad),
            valid: true,
        };
        if !self.airborne {
            return Ok(ActivationStep::Ready);
        }
        let frame = self.velocity_frame(now, [0.0; 3]);
        Ok(ActivationStep::Send { frame, prior })
    }

    /// Shapes one direct attitude and thrust setpoint. A new direct mode
    /// seeds its state from `seed_attitude_rad`.
    pub fn attitude_frame(
        &mut self,
        now: Duration,
        [roll_rad, pitch_rad, yaw_rad]: [f32; 3],
        thrust_norm: f32,
        seed_attitude_rad: [f32; 3],
    ) -> AttitudeOutcome {
        let quiet = StickOutcome {
            frame: None,
            constrained: false,
        };
        if self.in_quiet_interval(now) {
            return quiet;
        }
        let envelope = self.feel.envelope();
        let attitude_constrained = !roll_rad.is_finite()
            || !pitch_rad.is_finite()
            || roll_rad.abs() > envelope.direct_tilt_rad
            || pitch_rad.abs() > envelope.direct_tilt_rad
            || !yaw_rad.is_finite();
        let thrust_constrained = !thrust_norm.is_finite() || !(0.0..=1.0).contains(&thrust_norm);
        let throttle = finite_unit(thrust_norm) * 2.0 - 1.0;
        let takeoff_entry = !self.airborne;
        let refused = |constrained| StickOutcome {
            frame: None,
            constrained,
        };
        if !self.takeoff_requested(throttle) {
            return refused(attitude_constrained || thrust_constrained);
        }
        self.ensure_heading(seed_attitude_rad[2]);
        let changed_mode = self.select_mode(UplinkMode::Direct);
        let seed_constrained = changed_mode
            && !self.feel.is_legacy()
            && self
                .feel
                .seed_direct([seed_attitude_rad[0], seed_attitude_rad[1]], takeoff_entry);
        let dt_s = self.frame_dt(now);
        let demand = self.feel.step_direct(roll_rad, pitch_rad, throttle, dt_s);
        let input_constrained = attitude_constrained || thrust_constrained || seed_constrained;
        if !self.commit_direct_takeoff(demand.thrust) {
            return refused(demand.constrained || input_constrained);
        }
        let yaw_constrained = if self.feel.is_legacy() {
            if yaw_rad.is_finite() {
                self.heading.heading_rad = wrap_pi(yaw_rad);
            }
            !yaw_rad.is_finite()
        } else {
            self.limit_direct_heading(yaw_rad, dt_s)
        };
        let target = AttitudeTargetKind::from_euler(
            demand.roll_rad,
            demand.pitch_rad,
            self.heading.heading_rad,
        );
        let frame = self.attitude_target(now, target, demand.thrust);
        StickOutcome {
            frame: Some(frame),
            constrained: demand.constrained || input_constrained || yaw_constrained,
        }
    }

    /// Shapes one stick frame into a velocity, brake, or hold setpoint.
    pub fn stick_frame(&mut self, now: Duration, stick: &StickFrame) -> StickOutcome {
        if self.in_quiet_interval(now) {
            return StickOutcome {
                frame: None,
                constrained: false,
            };
        }
        let input_constrained = [stick.roll, stick.pitch, stick.throttle, stick.yaw]
            .iter()
            .any(|axis| !axis.is_finite() || !(-1.0..=1.0).contains(axis));
        let takeoff_entry = !self.airborne;
        if !self.takeoff_requested(stick.throttle) {
            return StickOutcome {
                frame: None,
                constrained: input_constrained,
            };
        }
        self.ensure_heading(stick.current_yaw_rad);
        let changed_mode = self.select_mode(UplinkMode::Normal);
        let seed_constrained = changed_mode
            && !self.feel.is_legacy()
            && self
                .feel
                .seed_normal_velocity(stick.current_yaw_rad, stick.current_vel_ned_mps);
        if changed_mode && takeoff_entry && !self.feel.is_legacy() {
            self.feel.seed_normal_takeoff();
        }
        let dt_s = self.frame_dt(now);
        let demand = self.feel.step_normal(
            [stick.roll, stick.pitch, stick.throttle, stick.yaw],
            stick.current_yaw_rad,
            dt_s,
        );
        let constrained = demand.constrained || input_constrained || seed_constrained;
        if !self.commit_normal_takeoff(demand.velocity_ned_mps[2]) {
            return StickOutcome {
                frame: None,
                constrained,
            };
        }
        let frame = if demand.settled {
            self.brake_or_hold(now, stick, dt_s)
        } else {
            self.hold_pos_ned = None;
            self.feel.reset_hold();
            self.heading.heading_rad =
                wrap_pi(self.heading.heading_rad + demand.yaw_rate_rps * dt_s);
            self.velocity_frame(now, demand.velocity_ned_mps)
        };
        StickOutcome {
            frame: Some(frame),
            constrained,
        }
    }

    /// One exact direct attitude frame, without shaping. The caller
    /// validates the setpoint against [`Self::envelope`].
    ///
    /// # Errors
    ///
    /// [`DirectRefusal`] inside the quiet interval or before the setpoint
    /// stream opens.
    pub fn exact_attitude_frame(
        &mut self,
        now: Duration,
        [roll_rad, pitch_rad, yaw_rad]: [f32; 3],
        collective_force: f32,
    ) -> Result<DirectFrame, DirectRefusal> {
        if self.in_quiet_interval(now) {
            return Err(DirectRefusal::QuietInterval);
        }
        if !self.airborne {
            return Err(DirectRefusal::StreamClosed);
        }
        let sequence = self.seq;
        let target = AttitudeTargetKind::from_euler(roll_rad, pitch_rad, yaw_rad);
        let frame = self.attitude_target(now, target, collective_force);
        Ok(DirectFrame {
            sequence,
            time_boot_ms: boot_ms(now),
            frame,
        })
    }

    fn attitude_target(
        &mut self,
        now: Duration,
        target: AttitudeTargetKind,
        thrust: f32,
    ) -> Vec<u8> {
        let frame = encode_attitude_target(
            self.seq,
            boot_ms(now),
            target,
            thrust,
            self.system_id,
            self.component_id,
        );
        self.seq = self.seq.wrapping_add(1);
        frame.to_vec()
    }

    fn brake_or_hold(&mut self, now: Duration, stick: &StickFrame, dt_s: f32) -> Vec<u8> {
        let speed_mps = magnitude(stick.current_vel_ned_mps);
        let accel_mps2 = magnitude(stick.current_accel_ned_mps2);
        let hold_ready =
            self.hold_pos_ned.is_some() || self.feel.hold_ready(speed_mps, accel_mps2, dt_s);
        if !hold_ready {
            return self.velocity_frame(now, [0.0; 3]);
        }
        let hold = *self.hold_pos_ned.get_or_insert(stick.current_pos_ned_m);
        let frame = encode_position_setpoint(
            self.seq,
            boot_ms(now),
            hold,
            self.heading.heading_rad,
            self.system_id,
            self.component_id,
        );
        self.seq = self.seq.wrapping_add(1);
        frame.to_vec()
    }

    fn select_mode(&mut self, mode: UplinkMode) -> bool {
        if self.feel.select_mode(mode) {
            self.last_frame = None;
            self.hold_pos_ned = None;
            return true;
        }
        false
    }

    fn frame_dt(&mut self, now: Duration) -> f32 {
        let dt_s = self
            .last_frame
            .map_or(0.0, |prior| now.saturating_sub(prior).as_secs_f32())
            .clamp(0.0, MAX_DT_S);
        self.last_frame = Some(now);
        dt_s
    }

    fn takeoff_requested(&mut self, throttle: f32) -> bool {
        if self.airborne {
            return true;
        }
        if !throttle.is_finite() || throttle <= self.feel.envelope().takeoff_input {
            self.reset_temporal_state();
            return false;
        }
        true
    }

    fn commit_normal_takeoff(&mut self, down_velocity_mps: f32) -> bool {
        if self.airborne {
            return true;
        }
        let envelope = self.feel.envelope();
        let minimum_climb_mps = envelope.takeoff_input * envelope.vertical_speed_mps;
        if !down_velocity_mps.is_finite() || -down_velocity_mps < minimum_climb_mps {
            return false;
        }
        self.airborne = true;
        tracing::info!(
            mode = "normal",
            "safe climb output opens the setpoint stream"
        );
        true
    }

    fn commit_direct_takeoff(&mut self, thrust: f32) -> bool {
        if self.airborne {
            return true;
        }
        let envelope = self.feel.envelope();
        let minimum = envelope.direct_hover_thrust
            + envelope.takeoff_input * (1.0 - envelope.direct_hover_thrust);
        if !thrust.is_finite() || thrust < minimum {
            return false;
        }
        self.airborne = true;
        tracing::info!(
            mode = "direct",
            "safe thrust output opens the setpoint stream"
        );
        true
    }

    fn limit_direct_heading(&mut self, requested_rad: f32, dt_s: f32) -> bool {
        if !requested_rad.is_finite() {
            return true;
        }
        let requested = wrap_pi(requested_rad);
        let delta = wrap_pi(requested - self.heading.heading_rad);
        let maximum = self.feel.envelope().yaw_rate_rps * dt_s;
        let applied = delta.clamp(-maximum, maximum);
        self.heading.heading_rad = wrap_pi(self.heading.heading_rad + applied);
        (applied - delta).abs() > f32::EPSILON
    }

    fn ensure_heading(&mut self, measured_rad: f32) {
        if !self.heading.valid && measured_rad.is_finite() {
            self.heading = HeadingState {
                heading_rad: wrap_pi(measured_rad),
                valid: true,
            };
        }
    }
}

fn magnitude(value: Option<[f32; 3]>) -> Option<f32> {
    value.map(|vector| {
        (vector[0] * vector[0] + vector[1] * vector[1] + vector[2] * vector[2]).sqrt()
    })
}

fn wrap_pi(rad: f32) -> f32 {
    let mut value = rad;
    while value > core::f32::consts::PI {
        value -= 2.0 * core::f32::consts::PI;
    }
    while value < -core::f32::consts::PI {
        value += 2.0 * core::f32::consts::PI;
    }
    value
}

fn finite_unit(value: f32) -> f32 {
    if value.is_finite() {
        value.clamp(0.0, 1.0)
    } else {
        0.5
    }
}
