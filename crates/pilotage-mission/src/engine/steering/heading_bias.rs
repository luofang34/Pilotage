//! The flight controller's heading bias: how far the heading it holds is
//! from the true heading it is commanded.
//!
//! A flight controller holds heading on its own estimate. When that
//! estimate's north is not true north (a magnetic reference with the
//! declination not applied, or a heading error), every commanded heading
//! is flown off by the same angle. Guidance then corrects the track only
//! through the cross-track law, which settles where the correction it asks
//! for equals the bias: a turn radius times the bias's tangent off the
//! track, 160 m for 10 degrees on a C172 at 2000 m. The navigator states
//! the true heading the aircraft holds, so the bias is that heading less
//! the one commanded, taken while the command is steady, and every command
//! is turned against it.

use navigate_contract::MonotonicNanos;

use super::super::MissionEngine;

/// How long the bias estimate takes to follow a change, seconds.
const BIAS_TIME_CONSTANT_S: f64 = 10.0;

/// The commanded heading rate below which the flight controller is taken
/// to hold the command, rad/s: well under a route turn's, above the slow
/// creep of a heading that settles on a track.
const STEADY_RATE_RPS: f64 = 0.02;

/// How long the command must have been steady before the held heading
/// says anything about a bias, seconds: the flight controller's heading
/// loop has settled by then.
const SETTLE_S: f64 = 5.0;

/// A difference between the held and the commanded heading beyond which
/// the sample is a transient or a fault, not a bias, radians.
const MAX_BIAS_RAD: f64 = 0.5;

/// The heading bias estimate and the command it is measured against.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub(super) struct HeadingBias {
    /// The bias, radians, positive when the held heading is right of the
    /// commanded one, and when it was last updated.
    estimate: Option<(f64, MonotonicNanos)>,
    /// The heading last commanded, radians, and when.
    previous_command: Option<(f64, MonotonicNanos)>,
    /// Since when the command has been steady.
    steady_since: Option<MonotonicNanos>,
}

impl MissionEngine {
    /// The heading to command for the aircraft to hold `heading` true.
    pub(super) fn unbiased_heading(&self, heading: f64) -> f64 {
        self.steering
            .heading_bias
            .estimate
            .map_or(heading, |(bias, _)| heading - bias)
    }

    /// Folds the heading the navigator states the aircraft holds at `now`
    /// into the bias estimate, while the last command has been steady.
    pub(super) fn update_heading_bias(&mut self, now: MonotonicNanos) {
        let bias = &mut self.steering.heading_bias;
        let (Some(held), Some((commanded, _))) = (self.last_yaw_rad, bias.previous_command) else {
            return;
        };
        let settled = bias.steady_since.is_some_and(|since| {
            now.elapsed_since(since)
                .is_some_and(|elapsed| elapsed.as_nanos() as f64 * 1e-9 >= SETTLE_S)
        });
        if !settled {
            return;
        }
        let sample = wrap(held - commanded);
        if !(sample.is_finite() && sample.abs() <= MAX_BIAS_RAD) {
            return;
        }
        let next = match bias.estimate {
            None => sample,
            Some((estimate, at)) => {
                let dt_s = now
                    .elapsed_since(at)
                    .map_or(0.0, |elapsed| elapsed.as_nanos() as f64 * 1e-9);
                estimate + wrap(sample - estimate) * (dt_s / BIAS_TIME_CONSTANT_S).min(1.0)
            }
        };
        bias.estimate = Some((next, now));
    }

    /// Records a heading command at `now`, and since when the command has
    /// been steady.
    pub(super) fn record_heading_command(&mut self, heading: f64, now: MonotonicNanos) {
        let bias = &mut self.steering.heading_bias;
        let steady = bias.previous_command.is_some_and(|(previous, at)| {
            let dt_s = now
                .elapsed_since(at)
                .map_or(0.0, |elapsed| elapsed.as_nanos() as f64 * 1e-9);
            dt_s > 0.0 && wrap(heading - previous).abs() <= STEADY_RATE_RPS * dt_s
        });
        bias.steady_since = if steady {
            bias.steady_since.or(Some(now))
        } else {
            None
        };
        bias.previous_command = Some((heading, now));
    }
}

/// `angle` wrapped into `[-π, π]`.
fn wrap(angle: f64) -> f64 {
    angle.sin().atan2(angle.cos())
}
