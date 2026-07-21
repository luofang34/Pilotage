//! PX4 gimbal-manager uplink (Gimbal Protocol v2): the primary-control
//! claim discipline, stick-rate streaming, the stale-demand cutoff,
//! and the neutral reset.
//!
//! Two typed lanes reach the FC's GCS instance through the telemetry
//! link's own socket (the instance retargets its stream to the last
//! peer that spoke, so a second socket would steal the telemetry
//! stream). The link task owns the socket's single MAVLink sequence and
//! does all encoding: claims and recenters ride an ordered command
//! queue, while rate demands ride a latest-value lane so a stalled link
//! coalesces stale demands instead of queueing them. PX4 silently
//! ignores GIMBAL_MANAGER_SET_ATTITUDE from a sender that does not hold
//! primary control, so the claim is re-asserted periodically while
//! demands flow.

use std::time::{Duration, Instant};

use tokio::sync::mpsc::Sender;
use tokio::sync::watch;
use tracing::{info, warn};

use pilotage_mavlink::codec::{GCS_COMPONENT_ID, GCS_SYSTEM_ID, GIMBAL_FLAGS_HORIZON_YAW_FOLLOW};
use pilotage_mavlink::{GimbalRateDemand, OutboundCommand};

/// Full-stick gimbal pitch rate (~46°/s).
const MAX_PITCH_RATE_RPS: f32 = 0.8;
/// Full-stick gimbal yaw rate (~46°/s).
const MAX_YAW_RATE_RPS: f32 = 0.8;
/// How often the primary-control claim is re-asserted while demands
/// flow: a claim lost to an FC restart or a competing GCS re-heals
/// within this bound.
const CLAIM_PERIOD: Duration = Duration::from_secs(1);
/// A demand stream silent longer than this is cut off with one
/// zero-rate setpoint. The FC's own stale-setpoint fallback is far
/// slower, so a stick release racing a dropped frame must be closed
/// out from this side.
const STALE_DEMAND_CUTOFF: Duration = Duration::from_millis(300);
/// How long idle (zero-rate) demands stay suppressed after a neutral
/// command: a rate setpoint replaces the manager's angle setpoint
/// within one control tick, so a continuous zero-rate keepalive stream
/// would race the one-shot recenter and usually win. Deliberate
/// nonzero demand breaks through immediately.
const NEUTRAL_SETTLE: Duration = Duration::from_millis(800);

/// MAV_CMD_DO_GIMBAL_MANAGER_PITCHYAW.
pub(crate) const CMD_GIMBAL_PITCHYAW: u16 = 1000;
/// MAV_CMD_DO_GIMBAL_MANAGER_CONFIGURE.
pub(crate) const CMD_GIMBAL_CONFIGURE: u16 = 1001;

/// The gimbal control clock; tests substitute a manually advanced
/// instant so the claim and cutoff cadences are exercised without
/// real-time sleeps.
#[derive(Debug)]
enum GimbalClock {
    System,
    #[cfg(test)]
    Manual(Instant),
}

impl GimbalClock {
    fn now(&self) -> Instant {
        match self {
            Self::System => Instant::now(),
            #[cfg(test)]
            Self::Manual(at) => *at,
        }
    }
}

/// The gimbal-manager command path over the link's typed lanes.
#[derive(Debug)]
pub struct Px4GimbalControl {
    commands: Sender<OutboundCommand>,
    rates: watch::Sender<Option<GimbalRateDemand>>,
    target_system: u8,
    target_component: u8,
    last_claim: Option<Instant>,
    last_demand: Option<Instant>,
    streaming: bool,
    neutral_settle_until: Option<Instant>,
    dropped_sends: u64,
    clock: GimbalClock,
}

impl Px4GimbalControl {
    /// Wires the control path onto a link's ordered command lane and
    /// latest-value rate lane.
    pub fn new(
        commands: Sender<OutboundCommand>,
        rates: watch::Sender<Option<GimbalRateDemand>>,
        target_system: u8,
        target_component: u8,
    ) -> Self {
        info!("PX4 gimbal-manager control path ready");
        Self {
            commands,
            rates,
            target_system,
            target_component,
            last_claim: None,
            last_demand: None,
            streaming: false,
            neutral_settle_until: None,
            dropped_sends: 0,
            clock: GimbalClock::System,
        }
    }

    /// Total commands or demands refused by a full or closed lane, for
    /// enactment-truth counter deltas.
    #[must_use]
    pub fn dropped_sends(&self) -> u64 {
        self.dropped_sends
    }

    /// Enqueues a reliable COMMAND_LONG. Returns false when the ordered
    /// lane is full or closed — a claim or recenter that never reached
    /// the wire must not be reported as applied.
    fn send_command(&mut self, command: u16, params: [f32; 7]) -> bool {
        let message = OutboundCommand {
            command,
            params,
            target_system: self.target_system,
            target_component: self.target_component,
        };
        if self.commands.try_send(message).is_err() {
            self.record_drop();
            return false;
        }
        true
    }

    fn record_drop(&mut self) {
        self.dropped_sends = self.dropped_sends.wrapping_add(1);
        if self.dropped_sends == 1 || self.dropped_sends.is_multiple_of(100) {
            warn!(
                dropped = self.dropped_sends,
                "gimbal uplink send dropped: lane full or closed"
            );
        }
    }

    /// Re-asserts the primary-control claim when due: CONFIGURE naming
    /// this codec's GCS identity as primary, leaving the secondary
    /// holder unchanged (-1 per the command convention). Returns false
    /// when a due claim could not be enqueued.
    fn claim_if_due(&mut self) -> bool {
        let now = self.clock.now();
        if self
            .last_claim
            .is_some_and(|at| now.duration_since(at) < CLAIM_PERIOD)
        {
            return true;
        }
        let sent = self.send_command(
            CMD_GIMBAL_CONFIGURE,
            [
                f32::from(GCS_SYSTEM_ID),
                f32::from(GCS_COMPONENT_ID),
                -1.0,
                -1.0,
                0.0,
                0.0,
                0.0,
            ],
        );
        if sent {
            self.last_claim = Some(now);
        }
        sent
    }

    /// Publishes the latest gimbal rate demand. Returns false when the
    /// rate lane has no receiver (the link was torn down).
    fn publish_rate(&mut self, pitch_rps: f32, yaw_rps: f32) -> bool {
        let demand = GimbalRateDemand {
            pitch_rps,
            yaw_rps,
            target_system: self.target_system,
            target_component: self.target_component,
        };
        if self.rates.send(Some(demand)).is_err() {
            self.record_drop();
            return false;
        }
        true
    }

    /// Converts one canonical gimbal stick frame (`[-1, 1]` pitch/yaw;
    /// pitch + = camera up, yaw + = camera right) into a rate demand.
    /// Idle (zero) demands inside the neutral settle window are dropped
    /// so the recenter's angle setpoint survives; real demand clears
    /// the window and steers immediately. Returns false when the demand
    /// could not be delivered.
    pub fn rate_demand(&mut self, pitch: f32, yaw: f32) -> bool {
        let idle = pitch == 0.0 && yaw == 0.0;
        if let Some(until) = self.neutral_settle_until {
            if idle && self.clock.now() < until {
                return true;
            }
            self.neutral_settle_until = None;
        }
        // Do not stream a rate the FC would drop: if a due claim could
        // not be enqueued, the sender does not hold primary control, so
        // report the demand rejected without publishing it.
        if !self.claim_if_due() {
            return false;
        }
        let published = self.publish_rate(pitch * MAX_PITCH_RATE_RPS, yaw * MAX_YAW_RATE_RPS);
        self.last_demand = Some(self.clock.now());
        self.streaming = true;
        published
    }

    /// Recenters the gimbal: an absolute zero-pitch/zero-yaw angle
    /// command under the horizon/yaw-follow flags, so the camera returns
    /// level and forward. Clears the rate lane so no stale demand
    /// overrides the angle setpoint, and opens a settle window against
    /// idle keepalives. Returns false when the command could not be
    /// enqueued.
    pub fn neutral(&mut self) -> bool {
        let claimed = self.claim_if_due();
        let sent = self.send_command(
            CMD_GIMBAL_PITCHYAW,
            [
                0.0,
                0.0,
                f32::NAN,
                f32::NAN,
                #[allow(clippy::cast_precision_loss)]
                {
                    GIMBAL_FLAGS_HORIZON_YAW_FOLLOW as f32
                },
                0.0,
                0.0,
            ],
        );
        // Drop any lingering rate demand so the link stops emitting rate
        // setpoints that would overwrite the recenter's angle target.
        self.rates.send(None).ok();
        self.neutral_settle_until = Some(self.clock.now() + NEUTRAL_SETTLE);
        self.streaming = false;
        info!("gimbal neutral commanded");
        claimed && sent
    }

    /// Sends a primary-control claim UNCONDITIONALLY (not debounced by
    /// [`Self::claim_if_due`]), so a link-loss stop re-asserts control even if a
    /// claim went out moments ago. Returns false when the command lane could not
    /// take it.
    fn reassert_primary_control(&mut self) -> bool {
        let now = self.clock.now();
        let sent = self.send_command(
            CMD_GIMBAL_CONFIGURE,
            [
                f32::from(GCS_SYSTEM_ID),
                f32::from(GCS_COMPONENT_ID),
                -1.0,
                -1.0,
                0.0,
                0.0,
                0.0,
            ],
        );
        if sent {
            self.last_claim = Some(now);
        }
        sent
    }

    /// Link-loss failsafe (BEST-EFFORT, queued — NOT FC-confirmed): re-asserts
    /// primary control and QUEUES a zero-rate setpoint to the FC's lanes, so a
    /// slew stops as promptly as the link allows instead of coasting to the
    /// `STALE_DEMAND_CUTOFF`. Unlike [`Self::neutral`] it holds the current
    /// pointing (zero RATE) rather than recentering — a failsafe stops the
    /// camera where it is, it does not slew it to level.
    ///
    /// The return value reports whether BOTH the claim and the zero-rate
    /// reached their local lanes — NOT whether the FC accepted them. There is
    /// no `MAV_CMD_DO_GIMBAL_MANAGER_CONFIGURE` acknowledgement or gimbal-status
    /// readback here, so the DECLARED safety net is independent of this queue:
    /// the FC/gimbal-manager's own setpoint-timeout failsafe ages an
    /// unrefreshed setpoint, and the host's `STALE_DEMAND_CUTOFF` re-sends a
    /// zero-rate. A `false` return (a lane full or closed) is surfaced as a
    /// typed enactment failure so the host counts it; it never means the FC
    /// confirmed a stop.
    pub fn queue_link_loss_stop(&mut self) -> bool {
        let claimed = self.reassert_primary_control();
        self.streaming = false;
        self.last_demand = None;
        let queued = self.publish_rate(0.0, 0.0);
        info!(
            claimed,
            queued, "gimbal link-loss zero-rate queued (best-effort)"
        );
        claimed && queued
    }

    /// Closes out a silent demand stream: one zero-rate setpoint when no
    /// demand has arrived within the cutoff, so a released stick or
    /// dropped control frame cannot leave the gimbal slewing. Call at
    /// telemetry-sampling cadence.
    pub fn maintain(&mut self) {
        if !self.streaming {
            return;
        }
        let now = self.clock.now();
        if self
            .last_demand
            .is_none_or(|at| now.duration_since(at) >= STALE_DEMAND_CUTOFF)
        {
            self.streaming = false;
            // Only claim the cutoff succeeded if the zero-rate setpoint
            // actually reached the lane; a closed lane means the link is
            // gone and there is nothing left to cut off.
            if self.publish_rate(0.0, 0.0) {
                info!("gimbal demand stream cut off with a zero-rate setpoint");
            }
        }
    }

    /// Whether a demand stream is active, for tests.
    #[cfg(test)]
    pub(crate) fn streaming(&self) -> bool {
        self.streaming
    }

    /// Switches to the manual clock, for tests.
    #[cfg(test)]
    pub(crate) fn use_manual_clock(&mut self) {
        self.clock = GimbalClock::Manual(Instant::now());
    }

    /// Advances the manual clock, for tests.
    #[cfg(test)]
    pub(crate) fn advance_clock(&mut self, by: Duration) {
        if let GimbalClock::Manual(at) = &mut self.clock {
            *at += by;
        }
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic)]
mod tests {
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
}
