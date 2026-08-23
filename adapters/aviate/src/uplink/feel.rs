//! Stateful demand shaping for the flight uplink.

use pilotage_control_feel::{
    AxisDemandShaper, AxisDynamics, DemandEnvelope, DemandPhase, FeelMode, HoldDetector,
    JerkLimitedAxis, ValidatedFlightFeelProfile,
};

/// The control family that owns the temporal state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum UplinkMode {
    /// Velocity and yaw-rate control.
    Normal,
    /// Attitude and thrust control.
    Direct,
}

/// One shaped velocity demand in the body frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct NormalDemand {
    pub(super) velocity_ned_mps: [f32; 3],
    pub(super) yaw_rate_rps: f32,
    pub(super) settled: bool,
    pub(super) constrained: bool,
}

/// One shaped direct attitude and thrust demand.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct DirectDemand {
    pub(super) roll_rad: f32,
    pub(super) pitch_rad: f32,
    pub(super) thrust: f32,
    pub(super) constrained: bool,
}

#[derive(Debug, Default)]
struct NormalShapers {
    roll: AxisDemandShaper,
    pitch: AxisDemandShaper,
    vertical: AxisDemandShaper,
    yaw: AxisDemandShaper,
}

impl NormalShapers {
    fn reset(&mut self) {
        self.roll.reset();
        self.pitch.reset();
        self.vertical.reset();
        self.yaw.reset();
    }

    fn seed(&mut self, body_velocity_mps: [f32; 3]) {
        self.roll.seed(body_velocity_mps[0]);
        self.pitch.seed(body_velocity_mps[1]);
        self.vertical.seed(body_velocity_mps[2]);
        self.yaw.seed(0.0);
    }

    fn seed_takeoff(&mut self, climb_mps: f32) {
        self.vertical.seed(climb_mps);
    }
}

#[derive(Debug, Default)]
struct DirectShapers {
    roll: JerkLimitedAxis,
    pitch: JerkLimitedAxis,
    thrust: JerkLimitedAxis,
}

#[derive(Debug, Default)]
struct LegacyHorizontalShapers {
    north: JerkLimitedAxis,
    east: JerkLimitedAxis,
}

impl LegacyHorizontalShapers {
    fn reset(&mut self) {
        self.north.reset();
        self.east.reset();
    }
}

impl DirectShapers {
    fn reset(&mut self) {
        self.roll.reset();
        self.pitch.reset();
        self.thrust.reset();
    }

    fn seed(&mut self, attitude_rad: [f32; 2], hover_thrust: f32) {
        self.roll.seed(attitude_rad[0]);
        self.pitch.seed(attitude_rad[1]);
        self.thrust.seed(hover_thrust);
    }
}

/// The profile and temporal state for one uplink.
#[derive(Debug)]
pub(super) struct UplinkFeel {
    profile: ValidatedFlightFeelProfile,
    normal: NormalShapers,
    direct: DirectShapers,
    legacy_horizontal: LegacyHorizontalShapers,
    hold: HoldDetector,
    mode: Option<UplinkMode>,
}

impl UplinkFeel {
    pub(super) fn new(profile: ValidatedFlightFeelProfile) -> Self {
        Self {
            profile,
            normal: NormalShapers::default(),
            direct: DirectShapers::default(),
            legacy_horizontal: LegacyHorizontalShapers::default(),
            hold: HoldDetector::default(),
            mode: None,
        }
    }

    pub(super) fn install(&mut self, profile: ValidatedFlightFeelProfile) {
        self.profile = profile;
        self.reset();
    }

    pub(super) fn envelope(&self) -> DemandEnvelope {
        self.profile.profile().envelope
    }

    #[cfg(test)]
    pub(super) fn validated_profile(&self) -> &ValidatedFlightFeelProfile {
        &self.profile
    }

    pub(super) fn is_legacy(&self) -> bool {
        self.profile.profile().mode == FeelMode::LegacyCompatibility
    }

    pub(super) fn select_mode(&mut self, mode: UplinkMode) -> bool {
        if self.mode == Some(mode) {
            return false;
        }
        self.reset();
        self.mode = Some(mode);
        true
    }

    pub(super) fn seed_direct(&mut self, attitude_rad: [f32; 2], takeoff_entry: bool) -> bool {
        let envelope = self.profile.profile().envelope;
        let limit = envelope.direct_tilt_rad;
        let finite = attitude_rad.map(|angle| if angle.is_finite() { angle } else { 0.0 });
        let constrained =
            finite != attitude_rad || finite.iter().any(|angle| !(-limit..=limit).contains(angle));
        self.direct.seed(
            [
                finite[0].clamp(-limit, limit),
                finite[1].clamp(-limit, limit),
            ],
            if takeoff_entry {
                envelope.direct_hover_thrust
                    + envelope.takeoff_input * (1.0 - envelope.direct_hover_thrust)
            } else {
                envelope.direct_hover_thrust
            },
        );
        constrained
    }

    pub(super) fn seed_normal_velocity(
        &mut self,
        current_yaw_rad: f32,
        velocity_ned_mps: Option<[f32; 3]>,
    ) -> bool {
        let Some(velocity) = velocity_ned_mps else {
            return false;
        };
        let envelope = self.profile.profile().envelope;
        let (sin_yaw, cos_yaw) = current_yaw_rad.sin_cos();
        let raw = [
            -velocity[0] * sin_yaw + velocity[1] * cos_yaw,
            velocity[0] * cos_yaw + velocity[1] * sin_yaw,
            -velocity[2],
        ];
        let bounded = [
            raw[0].clamp(
                -envelope.horizontal_speed_mps,
                envelope.horizontal_speed_mps,
            ),
            raw[1].clamp(
                -envelope.horizontal_speed_mps,
                envelope.horizontal_speed_mps,
            ),
            raw[2].clamp(-envelope.vertical_speed_mps, envelope.vertical_speed_mps),
        ];
        self.normal.seed(bounded);
        bounded != raw
    }

    pub(super) fn seed_normal_takeoff(&mut self) {
        let envelope = self.profile.profile().envelope;
        self.normal
            .seed_takeoff(envelope.takeoff_input * envelope.vertical_speed_mps);
    }

    pub(super) fn step_normal(
        &mut self,
        axes: [f32; 4],
        current_yaw_rad: f32,
        dt_s: f32,
    ) -> NormalDemand {
        if self.profile.profile().mode == FeelMode::LegacyCompatibility {
            return self.step_legacy_normal(axes, current_yaw_rad, dt_s);
        }
        self.step_profile_normal(axes, current_yaw_rad, dt_s)
    }

    fn step_profile_normal(
        &mut self,
        axes: [f32; 4],
        current_yaw_rad: f32,
        dt_s: f32,
    ) -> NormalDemand {
        let profile = self.profile.profile();
        let envelope = profile.envelope;
        let roll = step_axis(
            &mut self.normal.roll,
            axes[0],
            envelope.horizontal_speed_mps,
            dt_s,
            profile.horizontal,
        );
        let pitch = step_axis(
            &mut self.normal.pitch,
            axes[1],
            envelope.horizontal_speed_mps,
            dt_s,
            profile.horizontal,
        );
        let vertical = step_axis(
            &mut self.normal.vertical,
            axes[2],
            envelope.vertical_speed_mps,
            dt_s,
            profile.vertical,
        );
        let yaw = step_axis(
            &mut self.normal.yaw,
            axes[3],
            envelope.yaw_rate_rps,
            dt_s,
            profile.yaw,
        );
        let roll_value = roll.value.clamp(
            -envelope.horizontal_speed_mps,
            envelope.horizontal_speed_mps,
        );
        let pitch_value = pitch.value.clamp(
            -envelope.horizontal_speed_mps,
            envelope.horizontal_speed_mps,
        );
        let vertical_value = vertical
            .value
            .clamp(-envelope.vertical_speed_mps, envelope.vertical_speed_mps);
        let yaw_value = yaw
            .value
            .clamp(-envelope.yaw_rate_rps, envelope.yaw_rate_rps);
        let (sin_yaw, cos_yaw) = current_yaw_rad.sin_cos();
        NormalDemand {
            velocity_ned_mps: [
                pitch_value * cos_yaw - roll_value * sin_yaw,
                pitch_value * sin_yaw + roll_value * cos_yaw,
                -vertical_value,
            ],
            yaw_rate_rps: yaw_value,
            settled: [roll, pitch, vertical, yaw]
                .iter()
                .all(|axis| !axis.input_active && axis.value == 0.0),
            constrained: roll_value != roll.value
                || pitch_value != pitch.value
                || vertical_value != vertical.value
                || yaw_value != yaw.value,
        }
    }

    fn step_legacy_normal(
        &mut self,
        axes: [f32; 4],
        current_yaw_rad: f32,
        dt_s: f32,
    ) -> NormalDemand {
        let profile = self.profile.profile();
        let dt_s = dt_s.max(1.0 / 60.0);
        let curved = [
            profile.horizontal.curve.apply(axes[0]),
            profile.horizontal.curve.apply(axes[1]),
            profile.vertical.curve.apply(axes[2]),
            profile.yaw.curve.apply(axes[3]),
        ];
        let active = curved.iter().map(|value| value.abs()).fold(0.0, f32::max)
            > profile.horizontal.neutral.active_enter;
        if !active {
            self.normal.reset();
            self.legacy_horizontal.reset();
            return NormalDemand {
                velocity_ned_mps: [0.0; 3],
                yaw_rate_rps: 0.0,
                settled: true,
                constrained: false,
            };
        }
        let forward = curved[1] * profile.envelope.horizontal_speed_mps;
        let lateral = curved[0] * profile.envelope.horizontal_speed_mps;
        let (sin_yaw, cos_yaw) = current_yaw_rad.sin_cos();
        let north_target = forward * cos_yaw - lateral * sin_yaw;
        let east_target = forward * sin_yaw + lateral * cos_yaw;
        NormalDemand {
            velocity_ned_mps: [
                self.legacy_horizontal.north.step(
                    north_target,
                    dt_s,
                    DemandPhase::Apply,
                    profile.horizontal.dynamics,
                ),
                self.legacy_horizontal.east.step(
                    east_target,
                    dt_s,
                    DemandPhase::Apply,
                    profile.horizontal.dynamics,
                ),
                -curved[2] * profile.envelope.vertical_speed_mps,
            ],
            yaw_rate_rps: curved[3] * profile.envelope.yaw_rate_rps,
            settled: false,
            constrained: false,
        }
    }

    pub(super) fn step_direct(
        &mut self,
        roll_rad: f32,
        pitch_rad: f32,
        throttle: f32,
        dt_s: f32,
    ) -> DirectDemand {
        let profile = self.profile.profile();
        let envelope = profile.envelope;
        let limits = profile.direct;
        let tilt_dynamics = direct_dynamics(limits.tilt_rate_rps, limits.tilt_accel_rps2);
        let thrust_dynamics = direct_dynamics(limits.thrust_rate_per_s, limits.thrust_accel_per_s2);
        let roll_target = roll_rad.clamp(-envelope.direct_tilt_rad, envelope.direct_tilt_rad);
        let pitch_target = pitch_rad.clamp(-envelope.direct_tilt_rad, envelope.direct_tilt_rad);
        let thrust_target = map_thrust(throttle, envelope);
        if self.is_legacy() {
            return DirectDemand {
                roll_rad: finite_clamp(
                    roll_rad,
                    -envelope.direct_tilt_rad,
                    envelope.direct_tilt_rad,
                ),
                pitch_rad: finite_clamp(
                    pitch_rad,
                    -envelope.direct_tilt_rad,
                    envelope.direct_tilt_rad,
                ),
                thrust: thrust_target,
                constrained: direct_input_constrained(roll_rad, pitch_rad, throttle, envelope),
            };
        }
        let roll_phase = phase(&self.direct.roll, roll_target);
        let pitch_phase = phase(&self.direct.pitch, pitch_target);
        let thrust_phase = phase(&self.direct.thrust, thrust_target);
        let roll = self
            .direct
            .roll
            .step(roll_target, dt_s, roll_phase, tilt_dynamics);
        let pitch = self
            .direct
            .pitch
            .step(pitch_target, dt_s, pitch_phase, tilt_dynamics);
        let thrust = self
            .direct
            .thrust
            .step(thrust_target, dt_s, thrust_phase, thrust_dynamics);
        let bounded_thrust = thrust.clamp(envelope.direct_min_thrust, 1.0);
        DirectDemand {
            roll_rad: roll.clamp(-envelope.direct_tilt_rad, envelope.direct_tilt_rad),
            pitch_rad: pitch.clamp(-envelope.direct_tilt_rad, envelope.direct_tilt_rad),
            thrust: bounded_thrust,
            constrained: direct_input_constrained(roll_rad, pitch_rad, throttle, envelope)
                || bounded_thrust != thrust,
        }
    }

    pub(super) fn hold_ready(
        &mut self,
        speed_mps: Option<f32>,
        accel_mps2: Option<f32>,
        dt_s: f32,
    ) -> bool {
        self.hold
            .update(speed_mps, accel_mps2, dt_s, self.profile.profile().hold)
    }

    pub(super) fn reset_hold(&mut self) {
        self.hold.reset();
    }

    pub(super) fn reset(&mut self) {
        self.normal.reset();
        self.direct.reset();
        self.legacy_horizontal.reset();
        self.hold.reset();
        self.mode = None;
    }
}

fn step_axis(
    shaper: &mut AxisDemandShaper,
    normalized: f32,
    scale: f32,
    dt_s: f32,
    response: pilotage_control_feel::AxisResponse,
) -> pilotage_control_feel::ShapedDemand {
    shaper.step(normalized, scale, dt_s, response)
}

fn direct_dynamics(rate: f32, acceleration: f32) -> AxisDynamics {
    AxisDynamics {
        apply_accel: rate,
        release_accel: rate,
        apply_jerk: acceleration,
        release_jerk: acceleration,
        reversal_accel: rate,
        reversal_jerk: acceleration,
    }
}

fn phase(axis: &JerkLimitedAxis, target: f32) -> DemandPhase {
    let error = target - axis.value();
    if target == 0.0 {
        DemandPhase::Release
    } else if error.abs() > f32::EPSILON
        && axis.rate().abs() > f32::EPSILON
        && axis.rate().signum() != error.signum()
    {
        DemandPhase::Reversal
    } else {
        DemandPhase::Apply
    }
}

fn map_thrust(throttle: f32, envelope: DemandEnvelope) -> f32 {
    let throttle = if throttle.is_finite() {
        throttle.clamp(-1.0, 1.0)
    } else {
        0.0
    };
    if throttle >= 0.0 {
        envelope.direct_hover_thrust + throttle * (1.0 - envelope.direct_hover_thrust)
    } else {
        envelope.direct_hover_thrust
            + throttle * (envelope.direct_hover_thrust - envelope.direct_min_thrust)
    }
}

fn direct_input_constrained(
    roll_rad: f32,
    pitch_rad: f32,
    throttle: f32,
    envelope: DemandEnvelope,
) -> bool {
    !roll_rad.is_finite()
        || !pitch_rad.is_finite()
        || !throttle.is_finite()
        || roll_rad.abs() > envelope.direct_tilt_rad
        || pitch_rad.abs() > envelope.direct_tilt_rad
        || !(-1.0..=1.0).contains(&throttle)
}

fn finite_clamp(value: f32, minimum: f32, maximum: f32) -> f32 {
    if value.is_finite() {
        value.clamp(minimum, maximum)
    } else {
        0.0
    }
}
