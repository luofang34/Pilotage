//! The direct flight: hover under velocity control, switch to body rates
//! at the hover thrust, fly a yaw rate and a roll doublet, switch back,
//! and land.

use std::time::Duration;

use tracing::info;

use super::{Observed, Pilot, body_rates, start_pilot, velocity};

/// The most height either family switch may lose.
const SWITCH_LOSS_LIMIT_M: f64 = 1.0;
/// How far a measured rate may sit from the commanded one.
const RATE_TOLERANCE: f32 = 0.25;

impl Pilot {
    /// Flies body `rates` for `duration`. With `hold` set, the thrust
    /// closes a loop on that height the way a pilot holds altitude in
    /// acro; without it the thrust stays at the canonical hover, 0.5.
    async fn fly_rates(
        &mut self,
        rates: [f32; 3],
        duration: Duration,
        hold: Option<f64>,
    ) -> Vec<Observed> {
        let deadline = tokio::time::Instant::now() + duration;
        let mut seen = Vec::new();
        let mut thrust = 0.5_f32;
        while tokio::time::Instant::now() < deadline {
            if let Some(observed) = self.tick(body_rates(rates, thrust), vec![]).await {
                if let Some(height) = hold {
                    let error = height - observed.up;
                    thrust = (0.5 + 0.08 * error - 0.1 * observed.climb).clamp(0.3, 0.7) as f32;
                }
                seen.push(observed);
            }
        }
        seen
    }
}

/// The lowest height in `seen`.
fn lowest(seen: &[Observed]) -> f64 {
    seen.iter().map(|o| o.up).fold(f64::INFINITY, f64::min)
}

/// The mean of body-rate axis `axis` over the last `fraction` of `seen`.
fn late_mean_rate(seen: &[Observed], axis: usize, fraction: f64) -> f32 {
    let skip = ((seen.len() as f64) * (1.0 - fraction)) as usize;
    let late = &seen[skip.min(seen.len().saturating_sub(1))..];
    late.iter().map(|o| o.rates[axis]).sum::<f32>() / late.len().max(1) as f32
}

fn assert_rate(what: &str, measured: f32, commanded: f32) {
    let error = (measured - commanded).abs() / commanded.abs();
    info!(what, measured, commanded, error, "rate");
    assert!(
        error <= RATE_TOLERANCE,
        "{what}: flew {measured} rad/s for {commanded}"
    );
}

/// Arms, climbs above 4 m under velocity control, and hovers long enough
/// for ArduCopter to refine its hover thrust.
async fn climb_to_hover(pilot: &mut Pilot) -> Observed {
    let ground = pilot
        .fly_until("estimate ready", Duration::from_secs(120), None, |_| true)
        .await;
    pilot.arm().await;
    pilot
        .fly_until(
            "climb",
            Duration::from_secs(40),
            velocity(0.0, -1.5, 0.0),
            |o| o.up - ground.up >= 4.0,
        )
        .await;
    let mut hover = pilot
        .fly_until(
            "hover",
            Duration::from_secs(10),
            velocity(0.0, 0.0, 0.0),
            |o| o.climb.abs() < 0.1,
        )
        .await;
    // A few seconds of hover lets ArduCopter refine its hover thrust.
    for _ in 0..60 {
        if let Some(o) = pilot.tick(velocity(0.0, 0.0, 0.0), vec![]).await {
            hover = o;
        }
    }

    hover
}

pub(super) async fn flight(endpoint: std::net::SocketAddr) {
    let mut pilot = start_pilot(endpoint).await;
    let hover = climb_to_hover(&mut pilot).await;

    // Velocity -> direct, thrust at the canonical hover and no rates.
    let switched = pilot
        .fly_rates([0.0; 3], Duration::from_millis(1_500), None)
        .await;
    let into_direct_loss = hover.up - lowest(&switched);
    info!(into_direct_loss, "switch into direct");
    assert!(into_direct_loss <= SWITCH_LOSS_LIMIT_M);
    let height = hover.up;

    let yaw = pilot
        .fly_rates([0.0, 0.0, 0.8], Duration::from_millis(2_500), Some(height))
        .await;
    assert_rate("yaw rate", late_mean_rate(&yaw, 2, 0.4), 0.8);
    pilot
        .fly_rates([0.0; 3], Duration::from_millis(500), Some(height))
        .await;
    let roll = pilot
        .fly_rates([0.6, 0.0, 0.0], Duration::from_millis(400), Some(height))
        .await;
    assert_rate("roll rate", late_mean_rate(&roll, 0, 0.5), 0.6);
    pilot
        .fly_rates([-0.6, 0.0, 0.0], Duration::from_millis(400), Some(height))
        .await;
    let before_back = pilot
        .fly_rates([0.0; 3], Duration::from_millis(500), Some(height))
        .await;
    let before_back = before_back.last().copied().unwrap_or(hover);

    // Direct -> velocity: the FC holds the height it has.
    let mut back = Vec::new();
    for _ in 0..60 {
        if let Some(o) = pilot.tick(velocity(0.0, 0.0, 0.0), vec![]).await {
            back.push(o);
        }
    }
    let out_of_direct_loss = before_back.up - lowest(&back);
    info!(out_of_direct_loss, "switch back to velocity");
    assert!(out_of_direct_loss <= SWITCH_LOSS_LIMIT_M);

    pilot
        .tick(None, vec![pilotage_protocol::ControlAction::Disarm])
        .await;
    pilot
        .fly_until("land and disarm", Duration::from_secs(120), None, |o| {
            !o.armed
        })
        .await;
    info!(
        into_direct_loss,
        out_of_direct_loss,
        yaw_rate = late_mean_rate(&yaw, 2, 0.4),
        roll_rate = late_mean_rate(&roll, 0, 0.5),
        "direct flight complete"
    );
}
