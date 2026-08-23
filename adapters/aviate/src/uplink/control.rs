//! Normal and direct control frame construction.

use std::time::Instant;

use pilotage_mavlink::codec::{
    AttitudeTarget, encode_attitude_setpoint, encode_position_setpoint, encode_velocity_setpoint,
};

use super::feel::UplinkMode;
use super::{FlightUplink, MAX_DT_S};

impl FlightUplink {
    /// Sends one direct attitude and thrust setpoint.
    pub fn send_attitude_frame(
        &mut self,
        roll_rad: f32,
        pitch_rad: f32,
        yaw_rad: f32,
        thrust_norm: f32,
    ) -> bool {
        self.send_attitude_frame_seeded(
            roll_rad,
            pitch_rad,
            yaw_rad,
            thrust_norm,
            [0.0, 0.0, self.heading_sp_rad],
        )
    }

    /// Sends one direct setpoint and seeds a new direct mode from measured attitude.
    pub(crate) fn send_attitude_frame_seeded(
        &mut self,
        roll_rad: f32,
        pitch_rad: f32,
        yaw_rad: f32,
        thrust_norm: f32,
        seed_attitude_rad: [f32; 3],
    ) -> bool {
        let now = self.clock.now();
        if self.in_quiet_interval(now) {
            return false;
        }
        let envelope = self.feel.envelope();
        let attitude_constrained = !roll_rad.is_finite()
            || !pitch_rad.is_finite()
            || roll_rad.abs() > envelope.direct_tilt_rad
            || pitch_rad.abs() > envelope.direct_tilt_rad
            || !yaw_rad.is_finite();
        let thrust_constrained = !thrust_norm.is_finite() || !(0.0..=1.0).contains(&thrust_norm);
        let thrust_norm = finite_unit(thrust_norm);
        let throttle = thrust_norm * 2.0 - 1.0;
        let takeoff_entry = !self.airborne;
        if !self.takeoff_requested(throttle) {
            return attitude_constrained || thrust_constrained;
        }
        self.ensure_heading(seed_attitude_rad[2]);
        let changed_mode = self.select_mode(UplinkMode::Direct);
        let seed_constrained = changed_mode
            && !self.feel.is_legacy()
            && self
                .feel
                .seed_direct([seed_attitude_rad[0], seed_attitude_rad[1]], takeoff_entry);
        let (dt_s, shape_dt_s) = self.frame_times(now);
        let demand = self
            .feel
            .step_direct(roll_rad, pitch_rad, throttle, shape_dt_s);
        if !self.commit_direct_takeoff(demand.thrust) {
            return demand.constrained
                || attitude_constrained
                || thrust_constrained
                || seed_constrained;
        }
        let yaw_constrained = if self.feel.is_legacy() {
            if yaw_rad.is_finite() {
                self.heading_sp_rad = wrap_pi(yaw_rad);
                false
            } else {
                true
            }
        } else {
            self.limit_direct_heading(yaw_rad, dt_s)
        };
        let frame = encode_attitude_setpoint(
            self.seq,
            self.time_boot_ms(),
            AttitudeTarget {
                roll_rad: demand.roll_rad,
                pitch_rad: demand.pitch_rad,
                yaw_rad: self.heading_sp_rad,
                thrust: demand.thrust,
                system_id: self.expected_system_id,
                component_id: self.expected_component_id,
            },
        );
        self.send(&frame);
        demand.constrained
            || attitude_constrained
            || yaw_constrained
            || thrust_constrained
            || seed_constrained
    }

    /// Shapes one normalized stick frame and sends a velocity or hold setpoint.
    #[allow(clippy::too_many_arguments)]
    pub fn send_stick_frame(
        &mut self,
        roll: f32,
        pitch: f32,
        throttle: f32,
        yaw: f32,
        current_yaw_rad: f32,
        current_pos_ned_m: [f32; 3],
        current_vel_ned_mps: Option<[f32; 3]>,
        current_accel_ned_mps2: Option<[f32; 3]>,
    ) -> bool {
        let now = self.clock.now();
        if self.in_quiet_interval(now) {
            return false;
        }
        let input_constrained = [roll, pitch, throttle, yaw]
            .iter()
            .any(|axis| !axis.is_finite() || !(-1.0..=1.0).contains(axis));
        let takeoff_entry = !self.airborne;
        if !self.takeoff_requested(throttle) {
            return input_constrained;
        }
        self.ensure_heading(current_yaw_rad);
        let changed_mode = self.select_mode(UplinkMode::Normal);
        let seed_constrained = changed_mode
            && !self.feel.is_legacy()
            && self
                .feel
                .seed_normal_velocity(current_yaw_rad, current_vel_ned_mps);
        if changed_mode && takeoff_entry && !self.feel.is_legacy() {
            self.feel.seed_normal_takeoff();
        }
        let (dt_s, shape_dt_s) = self.frame_times(now);
        let demand =
            self.feel
                .step_normal([roll, pitch, throttle, yaw], current_yaw_rad, shape_dt_s);
        if !self.commit_normal_takeoff(demand.velocity_ned_mps[2]) {
            return demand.constrained || input_constrained || seed_constrained;
        }
        if demand.settled {
            self.send_brake_or_hold(
                current_pos_ned_m,
                current_vel_ned_mps,
                current_accel_ned_mps2,
                dt_s,
            );
            return demand.constrained || input_constrained || seed_constrained;
        }
        self.hold_pos_ned = None;
        self.feel.reset_hold();
        self.heading_sp_rad = wrap_pi(self.heading_sp_rad + demand.yaw_rate_rps * dt_s);
        let frame = encode_velocity_setpoint(
            self.seq,
            self.time_boot_ms(),
            demand.velocity_ned_mps,
            self.heading_sp_rad,
            self.expected_system_id,
            self.expected_component_id,
        );
        self.send(&frame);
        demand.constrained || input_constrained || seed_constrained
    }

    fn send_brake_or_hold(
        &mut self,
        current_pos_ned_m: [f32; 3],
        current_vel_ned_mps: Option<[f32; 3]>,
        current_accel_ned_mps2: Option<[f32; 3]>,
        dt_s: f32,
    ) {
        let speed_mps = magnitude(current_vel_ned_mps);
        let accel_mps2 = magnitude(current_accel_ned_mps2);
        let hold_ready =
            self.hold_pos_ned.is_some() || self.feel.hold_ready(speed_mps, accel_mps2, dt_s);
        if !hold_ready {
            let frame = encode_velocity_setpoint(
                self.seq,
                self.time_boot_ms(),
                [0.0; 3],
                self.heading_sp_rad,
                self.expected_system_id,
                self.expected_component_id,
            );
            self.send(&frame);
            return;
        }
        let hold = *self.hold_pos_ned.get_or_insert(current_pos_ned_m);
        let frame = encode_position_setpoint(
            self.seq,
            self.time_boot_ms(),
            hold,
            self.heading_sp_rad,
            self.expected_system_id,
            self.expected_component_id,
        );
        self.send(&frame);
    }

    fn select_mode(&mut self, mode: UplinkMode) -> bool {
        if self.feel.select_mode(mode) {
            self.last_frame = None;
            self.hold_pos_ned = None;
            return true;
        }
        false
    }

    fn frame_times(&mut self, now: Instant) -> (f32, f32) {
        let dt_s = self
            .last_frame
            .map_or(0.0, |prior| now.duration_since(prior).as_secs_f32())
            .clamp(0.0, MAX_DT_S);
        self.last_frame = Some(now);
        (dt_s, dt_s)
    }

    fn in_quiet_interval(&mut self, now: Instant) -> bool {
        if let Some(quiet) = self.quiet_until {
            if now < quiet {
                return true;
            }
            self.quiet_until = None;
        }
        false
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
        let delta = wrap_pi(requested - self.heading_sp_rad);
        let maximum = self.feel.envelope().yaw_rate_rps * dt_s;
        let applied = delta.clamp(-maximum, maximum);
        self.heading_sp_rad = wrap_pi(self.heading_sp_rad + applied);
        (applied - delta).abs() > f32::EPSILON
    }

    fn ensure_heading(&mut self, measured_rad: f32) {
        if !self.heading_valid && measured_rad.is_finite() {
            self.heading_sp_rad = wrap_pi(measured_rad);
            self.heading_valid = true;
        }
    }

    fn time_boot_ms(&self) -> u32 {
        self.clock
            .now()
            .saturating_duration_since(self.started)
            .as_millis() as u32
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
