//! Quad-X rigid-body dynamics in a local NED world with an FRD body.
//!
//! Motor order and spin follow Betaflight's default QUADX mixer: 0 rear
//! right (CW seen from above), 1 front right (CCW), 2 rear left (CCW),
//! 3 front left (CW). A CW propeller's reaction torque yaws the body
//! counter-clockwise, so speeding up motors 0 and 3 yaws left.

use crate::rotation::{Quat, Vec3};

/// Standard gravity, m/s².
pub const GRAVITY: f64 = 9.806_65;

/// Airframe constants.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct QuadParams {
    /// Mass, kg.
    pub mass: f64,
    /// Thrust of one motor at full command, N. Thrust grows with the
    /// square of the (lagged) command, as with rotor speed.
    pub max_thrust: f64,
    /// Forward and lateral offset of each motor from the centre, m.
    pub motor_offset: f64,
    /// Principal moments of inertia about body x, y, z, kg·m².
    pub inertia: Vec3,
    /// Reaction torque per newton of thrust, N·m/N.
    pub torque_per_thrust: f64,
    /// First-order motor time constant, s.
    pub motor_time_constant: f64,
    /// Linear drag coefficient, N/(m/s).
    pub linear_drag: f64,
    /// Quadratic drag coefficient, N/(m/s)².
    pub quadratic_drag: f64,
    /// Rotational damping, N·m/(rad/s).
    pub angular_damping: f64,
    /// Rate at which ground friction removes horizontal speed, 1/s.
    pub ground_friction: f64,
}

impl Default for QuadParams {
    /// An 0.8 kg, 6-inch-class quad with a 3:1 thrust-to-weight ratio, so
    /// it hovers near 57 % motor command.
    fn default() -> Self {
        Self {
            mass: 0.8,
            max_thrust: 6.0,
            motor_offset: 0.08,
            inertia: Vec3::new(0.002_5, 0.002_5, 0.004_5),
            torque_per_thrust: 0.016,
            motor_time_constant: 0.03,
            linear_drag: 0.15,
            quadratic_drag: 0.05,
            angular_damping: 0.000_5,
            ground_friction: 20.0,
        }
    }
}

impl QuadParams {
    /// The motor command that holds the airframe in a level hover.
    #[must_use]
    pub fn hover_command(&self) -> f64 {
        (self.mass * GRAVITY / (4.0 * self.max_thrust)).sqrt()
    }

    /// Body positions of the four motors in mixer order (FRD, z = 0).
    fn motor_positions(&self) -> [Vec3; 4] {
        let d = self.motor_offset;
        [
            Vec3::new(-d, d, 0.0),
            Vec3::new(d, d, 0.0),
            Vec3::new(-d, -d, 0.0),
            Vec3::new(d, -d, 0.0),
        ]
    }
}

/// Reaction-torque sign about body +z (down) per motor, mixer order.
const YAW_SIGN: [f64; 4] = [-1.0, 1.0, 1.0, -1.0];

/// The integrated state.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct QuadState {
    /// Position from the local origin, NED, m.
    pub position: Vec3,
    /// Velocity, NED, m/s.
    pub velocity: Vec3,
    /// Body (FRD) to NED attitude.
    pub attitude: Quat,
    /// Body rates, FRD, rad/s.
    pub rates: Vec3,
    /// Lagged motor commands, 0..1, mixer order.
    pub motors: [f64; 4],
    /// Specific force the last step produced, body FRD, m/s²: what an
    /// accelerometer measures (at rest, `-g` on z).
    pub specific_force: Vec3,
    /// Whether the airframe rests on the ground.
    pub on_ground: bool,
}

impl QuadState {
    /// At rest on the ground at `position` with heading `yaw` (radians).
    #[must_use]
    pub fn landed(position: Vec3, yaw: f64) -> Self {
        Self {
            position: Vec3::new(position.x, position.y, 0.0),
            attitude: Quat::from_euler(0.0, 0.0, yaw),
            specific_force: Vec3::new(0.0, 0.0, -GRAVITY),
            on_ground: true,
            ..Self::default()
        }
    }

    /// Advances the state by `dt` seconds under motor `commands` (0..1,
    /// mixer order; values outside are clamped, non-finite read as 0).
    pub fn step(&mut self, params: &QuadParams, commands: [f64; 4], dt: f64) {
        let blend = 1.0 - (-dt / params.motor_time_constant).exp();
        for (motor, command) in self.motors.iter_mut().zip(commands) {
            let command = if command.is_finite() {
                command.clamp(0.0, 1.0)
            } else {
                0.0
            };
            *motor += (command - *motor) * blend;
        }
        let thrusts = self.motors.map(|m| params.max_thrust * m * m);
        let total_thrust: f64 = thrusts.iter().sum();
        let velocity_before = self.velocity;
        self.integrate_translation(params, total_thrust, dt);
        let grounded = self.resolve_ground(params, total_thrust, dt);
        if grounded {
            self.settle_attitude();
        } else {
            self.integrate_rotation(params, &thrusts, dt);
        }
        let acceleration = (self.velocity - velocity_before) * (1.0 / dt);
        let specific_force_world = acceleration - Vec3::new(0.0, 0.0, GRAVITY);
        self.specific_force = self.attitude.rotate_inverse(specific_force_world);
    }

    fn integrate_translation(&mut self, params: &QuadParams, total_thrust: f64, dt: f64) {
        let thrust_world = self.attitude.rotate(Vec3::new(0.0, 0.0, -total_thrust));
        let speed = self.velocity.norm();
        let drag = -(self.velocity * (params.linear_drag + params.quadratic_drag * speed));
        let force = thrust_world + drag + Vec3::new(0.0, 0.0, params.mass * GRAVITY);
        self.velocity = self.velocity + force * (dt / params.mass);
        self.position = self.position + self.velocity * dt;
    }

    /// Applies ground contact: the ground holds the airframe up and
    /// friction stops it sliding. Returns whether it rests on the ground
    /// with too little thrust to lift off.
    fn resolve_ground(&mut self, params: &QuadParams, total_thrust: f64, dt: f64) -> bool {
        if self.position.z < 0.0 {
            self.on_ground = false;
            return false;
        }
        self.position.z = 0.0;
        if self.velocity.z > 0.0 {
            self.velocity.z = 0.0;
        }
        let friction = (-params.ground_friction * dt).exp();
        self.velocity.x *= friction;
        self.velocity.y *= friction;
        self.on_ground = total_thrust < params.mass * GRAVITY;
        self.on_ground
    }

    /// Lays the airframe flat on its feet, keeping its heading.
    fn settle_attitude(&mut self) {
        let (_, _, yaw) = self.attitude.to_euler();
        self.attitude = Quat::from_euler(0.0, 0.0, yaw);
        self.rates = Vec3::ZERO;
        self.velocity = Vec3::ZERO;
    }

    fn integrate_rotation(&mut self, params: &QuadParams, thrusts: &[f64; 4], dt: f64) {
        let mut torque = -(self.rates * params.angular_damping);
        for ((position, thrust), yaw_sign) in
            params.motor_positions().iter().zip(thrusts).zip(YAW_SIGN)
        {
            torque = torque + position.cross(Vec3::new(0.0, 0.0, -thrust));
            torque.z += yaw_sign * params.torque_per_thrust * thrust;
        }
        let inertia = params.inertia;
        let momentum = Vec3::new(
            inertia.x * self.rates.x,
            inertia.y * self.rates.y,
            inertia.z * self.rates.z,
        );
        let net = torque - self.rates.cross(momentum);
        let rate_change = Vec3::new(net.x / inertia.x, net.y / inertia.y, net.z / inertia.z);
        self.rates = self.rates + rate_change * dt;
        self.attitude = self.attitude.integrate(self.rates, dt);
    }
}

#[cfg(test)]
mod tests;
