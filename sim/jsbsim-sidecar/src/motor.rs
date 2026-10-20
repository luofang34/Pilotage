//! The x500's motors and propellers: PX4's normalized motor command sets
//! a rotor speed between idle and maximum, the rotor follows it with a
//! first-order lag, and thrust and drag torque grow with its square.
//!
//! The constants are those of PX4's Gazebo x500 model, so a JSBSim x500
//! answers PX4's commands the way the Gazebo one did.

/// One rotor's constants.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MotorConstants {
    /// Rotor speed at a zero command while armed, rad/s.
    pub idle_rad_s: f64,
    /// Rotor speed at a full command, rad/s.
    pub max_rad_s: f64,
    /// Thrust per squared rotor speed, N·s².
    pub thrust_per_rad2: f64,
    /// Drag torque per newton of thrust, m.
    pub torque_per_thrust_m: f64,
    /// Time constant of a speed increase, s.
    pub spin_up_s: f64,
    /// Time constant of a speed decrease, s.
    pub spin_down_s: f64,
}

/// The x500's rotor (Gazebo `x500` model and airframe 4001 limits).
pub const X500_MOTOR: MotorConstants = MotorConstants {
    idle_rad_s: 150.0,
    max_rad_s: 1000.0,
    thrust_per_rad2: 8.548_58e-6,
    torque_per_thrust_m: 0.016,
    spin_up_s: 0.0125,
    spin_down_s: 0.025,
};

/// The yaw-torque sign of each rotor in PX4's quad-X order (front right,
/// rear left, front left, rear right): a positive `CA_ROTORn_KM` rotor
/// spins counter-clockwise seen from above and yaws the body clockwise.
pub const QUAD_X_YAW_SIGN: [f64; 4] = [1.0, 1.0, -1.0, -1.0];

/// What the four rotors push and twist with.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RotorOutput {
    /// Each rotor's thrust, N, along body up.
    pub thrust_n: [f64; 4],
    /// The summed drag torque about body z (down), N·m.
    pub yaw_torque_nm: f64,
}

/// Four rotors' speeds.
#[derive(Debug, Clone, PartialEq)]
pub struct QuadMotors {
    constants: MotorConstants,
    speed_rad_s: [f64; 4],
}

impl QuadMotors {
    /// Stopped rotors.
    pub fn new(constants: MotorConstants) -> Self {
        Self {
            constants,
            speed_rad_s: [0.0; 4],
        }
    }

    /// Advances the rotors `dt_s` toward `commands` (each in [0, 1]) and
    /// returns their thrust and torque. A disarmed vehicle's rotors stop.
    pub fn step(&mut self, commands: [f64; 4], armed: bool, dt_s: f64) -> RotorOutput {
        let c = self.constants;
        let mut thrust_n = [0.0; 4];
        let mut yaw_torque_nm = 0.0;
        for (i, speed) in self.speed_rad_s.iter_mut().enumerate() {
            let target = if armed {
                let command = if commands[i].is_finite() {
                    commands[i].clamp(0.0, 1.0)
                } else {
                    0.0
                };
                c.idle_rad_s + (c.max_rad_s - c.idle_rad_s) * command
            } else {
                0.0
            };
            let tau = if target > *speed {
                c.spin_up_s
            } else {
                c.spin_down_s
            };
            *speed += (target - *speed) * (1.0 - (-dt_s / tau).exp());
            thrust_n[i] = c.thrust_per_rad2 * *speed * *speed;
            yaw_torque_nm += QUAD_X_YAW_SIGN[i] * c.torque_per_thrust_m * thrust_n[i];
        }
        RotorOutput {
            thrust_n,
            yaw_torque_nm,
        }
    }
}

#[cfg(test)]
mod tests;
