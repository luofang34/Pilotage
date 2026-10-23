//! A simple battery energy model.
//!
//! The draw is the idle power on the ground with the motors running, and
//! in the air the hover power plus a part that grows with the square of
//! the ground speed. The voltage falls linearly with the used energy.

use crate::config::BatteryConfig;

/// The motor state the draw depends on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Draw {
    /// The motors are stopped.
    Off,
    /// The motors run at idle on the ground.
    Idle,
    /// The vehicle flies.
    Flying,
}

/// The pack state.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Battery {
    config: BatteryConfig,
    consumed_j: f64,
    power_w: f64,
}

impl Battery {
    /// A full pack.
    #[must_use]
    pub fn full(config: BatteryConfig) -> Self {
        Self {
            config,
            consumed_j: 0.0,
            power_w: 0.0,
        }
    }

    /// Draws energy for `dt_s` seconds.
    pub fn step(&mut self, draw: Draw, ground_speed_mps: f64, dt_s: f64) {
        let config = &self.config;
        self.power_w = match draw {
            Draw::Off => 0.0,
            Draw::Idle => config.idle_power_w,
            Draw::Flying => {
                let ratio = if config.cruise_speed_mps > 0.0 {
                    ground_speed_mps / config.cruise_speed_mps
                } else {
                    0.0
                };
                config.hover_power_w
                    + (config.cruise_power_w - config.hover_power_w) * ratio * ratio
            }
        };
        let capacity_j = self.capacity_j();
        self.consumed_j = (self.consumed_j + self.power_w * dt_s).min(capacity_j);
    }

    fn capacity_j(&self) -> f64 {
        self.config.capacity_wh.max(0.0) * 3_600.0
    }

    /// Remaining energy as a fraction of a full pack, 0 to 1.
    #[must_use]
    pub fn remaining_fraction(&self) -> f64 {
        let capacity = self.capacity_j();
        if capacity <= 0.0 {
            return 0.0;
        }
        (1.0 - self.consumed_j / capacity).clamp(0.0, 1.0)
    }

    /// Pack voltage, volts.
    #[must_use]
    pub fn voltage_v(&self) -> f64 {
        let config = &self.config;
        config.empty_voltage_v
            + (config.full_voltage_v - config.empty_voltage_v) * self.remaining_fraction()
    }

    /// Current out of the pack, amperes.
    #[must_use]
    pub fn current_a(&self) -> f64 {
        let voltage = self.voltage_v();
        if voltage > 0.0 {
            self.power_w / voltage
        } else {
            0.0
        }
    }

    /// Energy used since the pack was full, joules.
    #[must_use]
    pub fn consumed_j(&self) -> f64 {
        self.consumed_j
    }

    /// Seconds until empty at the present draw, or `None` with no draw.
    #[must_use]
    pub fn time_remaining_s(&self) -> Option<u32> {
        if self.power_w <= 0.0 {
            return None;
        }
        let seconds = (self.capacity_j() - self.consumed_j).max(0.0) / self.power_w;
        Some(seconds.min(f64::from(u32::MAX)) as u32)
    }

    /// Whether the pack is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.remaining_fraction() <= 0.0
    }
}

#[cfg(test)]
mod tests;
