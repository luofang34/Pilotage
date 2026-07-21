//! `Px4GimbalControl` unit tests: the primary-control claim discipline,
//! stick-rate streaming, the stale-demand cutoff, the neutral reset, and the
//! link-loss stop (including the dropped-stop fault injection).
#![allow(clippy::expect_used, clippy::panic)]

use std::time::Duration;

use pilotage_mavlink::{GimbalRateDemand, OutboundCommand};
use tokio::sync::{mpsc, watch};

use super::Px4GimbalControl;

struct Lanes {
    commands: mpsc::Receiver<OutboundCommand>,
    rates: watch::Receiver<Option<GimbalRateDemand>>,
}

fn control() -> (Px4GimbalControl, Lanes) {
    let (command_tx, command_rx) = mpsc::channel(16);
    let (rate_tx, rate_rx) = watch::channel(None);
    let mut control = Px4GimbalControl::new(command_tx, rate_tx, 1, 1);
    control.use_manual_clock();
    (
        control,
        Lanes {
            commands: command_rx,
            rates: rate_rx,
        },
    )
}

fn next_command(lanes: &mut Lanes) -> u16 {
    lanes.commands.try_recv().expect("queued command").command
}

fn latest_rate(lanes: &mut Lanes) -> Option<GimbalRateDemand> {
    *lanes.rates.borrow_and_update()
}

#[test]
fn a_dropped_link_loss_stop_sends_nothing_so_px4_is_the_sole_failsafe() {
    let (control, mut lanes) = control();
    let mut control = control.with_dropped_link_loss_stop(true);
    // Prime a nonzero slew: the claim and a nonzero rate reach the lanes.
    assert!(control.rate_demand(0.5, 0.0));
    while lanes.commands.try_recv().is_ok() {} // drain the claim(s)
    let primed = latest_rate(&mut lanes).expect("a nonzero rate was streaming");
    assert!(primed.pitch_rps != 0.0);

    // Link loss with the stop DROPPED: nothing leaves the host, so PX4's own
    // setpoint-timeout is the only thing that will stop the slew. The latch
    // still engages (returns true), and the gimbal keeps its last rate.
    assert!(control.queue_link_loss_stop());
    assert!(
        lanes.commands.try_recv().is_err(),
        "a dropped stop sends no claim command"
    );
    assert!(
        !lanes.rates.has_changed().expect("rate lane open"),
        "a dropped stop publishes no zero-rate; the gimbal keeps its last rate until PX4 times out"
    );
}

#[test]
fn first_demand_claims_before_streaming() {
    let (mut control, mut lanes) = control();
    assert!(control.rate_demand(0.5, -0.25));
    assert_eq!(next_command(&mut lanes), super::CMD_GIMBAL_CONFIGURE);
    let rate = latest_rate(&mut lanes).expect("rate published");
    assert!(rate.pitch_rps > 0.0 && rate.yaw_rps < 0.0);
    assert!(control.streaming());
}

#[test]
fn claim_reasserts_only_after_its_period() {
    let (mut control, mut lanes) = control();
    control.rate_demand(0.1, 0.0);
    assert_eq!(next_command(&mut lanes), super::CMD_GIMBAL_CONFIGURE);
    // Within the period: demands stream without a fresh claim.
    control.advance_clock(Duration::from_millis(400));
    control.rate_demand(0.2, 0.0);
    assert!(
        lanes.commands.try_recv().is_err(),
        "no re-claim within period"
    );
    // Past the period: the claim re-heals with the next demand.
    control.advance_clock(Duration::from_millis(700));
    control.rate_demand(0.3, 0.0);
    assert_eq!(next_command(&mut lanes), super::CMD_GIMBAL_CONFIGURE);
}

#[test]
fn stale_demand_stream_cuts_off_with_one_zero_rate() {
    let (mut control, mut lanes) = control();
    control.rate_demand(1.0, 1.0);
    assert_eq!(next_command(&mut lanes), super::CMD_GIMBAL_CONFIGURE);
    latest_rate(&mut lanes);
    // Fresh demands keep the stream open.
    control.advance_clock(Duration::from_millis(100));
    control.maintain();
    assert!(
        !lanes.rates.has_changed().unwrap_or(false),
        "no cutoff while fresh"
    );
    // Silence past the cutoff closes the stream with zero rates.
    control.advance_clock(Duration::from_millis(400));
    control.maintain();
    let rate = latest_rate(&mut lanes).expect("cutoff demand");
    assert_eq!((rate.pitch_rps, rate.yaw_rps), (0.0, 0.0));
    assert!(!control.streaming());
}

#[test]
fn neutral_sends_the_pitchyaw_command_and_clears_rates() {
    let (mut control, mut lanes) = control();
    control.rate_demand(0.5, 0.0);
    assert_eq!(next_command(&mut lanes), super::CMD_GIMBAL_CONFIGURE);
    latest_rate(&mut lanes);
    assert!(control.neutral());
    assert_eq!(next_command(&mut lanes), super::CMD_GIMBAL_PITCHYAW);
    assert_eq!(
        latest_rate(&mut lanes),
        None,
        "rate lane cleared for the recenter"
    );
}

#[test]
fn neutral_settle_window_holds_off_idle_keepalives() {
    let (mut control, mut lanes) = control();
    control.neutral();
    assert_eq!(next_command(&mut lanes), super::CMD_GIMBAL_CONFIGURE);
    assert_eq!(next_command(&mut lanes), super::CMD_GIMBAL_PITCHYAW);
    latest_rate(&mut lanes);
    // Idle keepalives inside the window do not repopulate the lane.
    control.rate_demand(0.0, 0.0);
    control.advance_clock(Duration::from_millis(400));
    control.rate_demand(0.0, 0.0);
    assert!(
        !lanes.rates.has_changed().unwrap_or(false),
        "the settle window must keep the rate lane quiet"
    );
    // Past the window the keepalive stream resumes.
    control.advance_clock(Duration::from_millis(500));
    control.rate_demand(0.0, 0.0);
    assert!(lanes.rates.has_changed().unwrap_or(false));
}

#[test]
fn real_demand_breaks_through_the_settle_window() {
    let (mut control, mut lanes) = control();
    control.neutral();
    assert_eq!(next_command(&mut lanes), super::CMD_GIMBAL_CONFIGURE);
    assert_eq!(next_command(&mut lanes), super::CMD_GIMBAL_PITCHYAW);
    latest_rate(&mut lanes);
    control.advance_clock(Duration::from_millis(100));
    assert!(control.rate_demand(0.5, 0.0));
    let rate = latest_rate(&mut lanes).expect("deliberate demand steers immediately");
    assert!(rate.pitch_rps > 0.0);
}

#[test]
fn a_closed_command_lane_reports_a_dropped_send() {
    let (command_tx, command_rx) = mpsc::channel(1);
    let (rate_tx, _rate_rx) = watch::channel(None);
    let mut control = Px4GimbalControl::new(command_tx, rate_tx, 1, 1);
    control.use_manual_clock();
    // Closing the lane (receiver dropped) makes the due claim's
    // try_send fail, so the recenter cannot be reported as applied.
    drop(command_rx);
    assert!(
        !control.neutral(),
        "a command that never reached the wire is not applied"
    );
    assert!(control.dropped_sends() >= 1);
}
