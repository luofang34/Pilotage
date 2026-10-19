//! Keeps simulation time near wall time.
//!
//! In lockstep the flight controller cannot run ahead of the simulator,
//! but either can fall behind: a loaded machine starves PX4, and every
//! step then waits for its answer. Once the lag passes a bound, the pacer
//! gives it up instead of making it good: the simulator then runs at the
//! paced rate again rather than faster than real time, which would
//! compress the flight controller's and the operator's time alike.

use std::time::{Duration, Instant};

/// The lag the pacer makes good; more than this, it gives up.
pub const MAX_LAG: Duration = Duration::from_millis(100);

/// Paces simulation time against wall time.
#[derive(Debug, Clone, Copy)]
pub struct Pacer {
    realtime_factor: f64,
    anchor_wall: Instant,
    anchor_sim_s: f64,
}

impl Pacer {
    /// A pacer at `realtime_factor` simulation seconds per wall second
    /// (0 runs unpaced), anchored at `now` and `sim_s`.
    pub fn new(realtime_factor: f64, now: Instant, sim_s: f64) -> Self {
        Self {
            realtime_factor,
            anchor_wall: now,
            anchor_sim_s: sim_s,
        }
    }

    /// How long to sleep before the step after `sim_s`, given the time is
    /// `now`. A lag past [`MAX_LAG`] moves the anchor to now.
    pub fn wait(&mut self, sim_s: f64, now: Instant) -> Option<Duration> {
        if self.realtime_factor <= 0.0 {
            return None;
        }
        let ahead_s = (sim_s - self.anchor_sim_s) / self.realtime_factor;
        let due = self.anchor_wall + Duration::from_secs_f64(ahead_s.max(0.0));
        if now > due + MAX_LAG {
            self.anchor_wall = now;
            self.anchor_sim_s = sim_s;
            return None;
        }
        due.checked_duration_since(now)
    }
}

#[cfg(test)]
mod tests;
