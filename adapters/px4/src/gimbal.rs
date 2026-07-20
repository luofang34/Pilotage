//! PX4 gimbal-manager uplink (Gimbal Protocol v2): the primary-control
//! claim discipline, stick-rate streaming, the stale-demand cutoff,
//! and the neutral reset.
//!
//! Frames leave through the telemetry link's own socket (the FC's GCS
//! instance retargets its stream to the last peer that spoke, so a
//! second socket would steal the telemetry stream). PX4 silently
//! ignores GIMBAL_MANAGER_SET_ATTITUDE from a sender that does not
//! hold primary control, so the claim is re-asserted periodically
//! while demands flow instead of being trusted once.

use std::time::{Duration, Instant};

use tokio::sync::mpsc::Sender;
use tracing::{info, warn};

use pilotage_mavlink::codec::{
    GCS_COMPONENT_ID, GCS_SYSTEM_ID, GIMBAL_FLAGS_HORIZON_YAW_FOLLOW, encode_command_long,
    encode_gimbal_rate_setpoint,
};

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

/// The gimbal-manager command path riding the telemetry link's socket.
#[derive(Debug)]
pub struct Px4GimbalControl {
    outbound: Sender<Vec<u8>>,
    seq: u8,
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
    /// Wires the control path onto a link's outbound sender.
    pub fn new(outbound: Sender<Vec<u8>>, target_system: u8, target_component: u8) -> Self {
        info!("PX4 gimbal-manager control path ready");
        Self {
            outbound,
            seq: 0,
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

    /// Total frames refused by the outbound queue, for enactment-truth
    /// counter deltas.
    #[must_use]
    pub fn dropped_sends(&self) -> u64 {
        self.dropped_sends
    }

    fn send(&mut self, frame: &[u8]) {
        if self.outbound.try_send(frame.to_vec()).is_err() {
            self.dropped_sends = self.dropped_sends.wrapping_add(1);
            if self.dropped_sends == 1 || self.dropped_sends.is_multiple_of(100) {
                warn!(
                    dropped = self.dropped_sends,
                    "gimbal uplink frame dropped: outbound queue unavailable"
                );
            }
        }
        self.seq = self.seq.wrapping_add(1);
    }

    /// Re-asserts the primary-control claim when due: CONFIGURE naming
    /// this codec's GCS identity as primary, leaving the secondary
    /// holder unchanged (-1 per the command convention).
    fn claim_if_due(&mut self) {
        let now = self.clock.now();
        if self
            .last_claim
            .is_some_and(|at| now.duration_since(at) < CLAIM_PERIOD)
        {
            return;
        }
        self.last_claim = Some(now);
        let frame = encode_command_long(
            self.seq,
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
            self.target_system,
            self.target_component,
        );
        self.send(&frame);
    }

    /// Converts one canonical gimbal stick frame (`[-1, 1]` pitch/yaw;
    /// pitch + = camera up, yaw + = camera right) into a rate demand.
    /// Idle (zero) demands inside the neutral settle window are dropped
    /// so the recenter's angle setpoint survives; real demand clears
    /// the window and steers immediately.
    pub fn rate_demand(&mut self, pitch: f32, yaw: f32) {
        let idle = pitch == 0.0 && yaw == 0.0;
        if let Some(until) = self.neutral_settle_until {
            if idle && self.clock.now() < until {
                return;
            }
            self.neutral_settle_until = None;
        }
        self.claim_if_due();
        let frame = encode_gimbal_rate_setpoint(
            self.seq,
            pitch * MAX_PITCH_RATE_RPS,
            yaw * MAX_YAW_RATE_RPS,
            self.target_system,
            self.target_component,
        );
        self.send(&frame);
        self.last_demand = Some(self.clock.now());
        self.streaming = true;
    }

    /// Recenters the gimbal: an absolute zero-pitch/zero-yaw angle
    /// command under the horizon/yaw-follow flags, so the camera
    /// returns level and forward.
    pub fn neutral(&mut self) {
        self.claim_if_due();
        let frame = encode_command_long(
            self.seq,
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
            self.target_system,
            self.target_component,
        );
        self.send(&frame);
        self.neutral_settle_until = Some(self.clock.now() + NEUTRAL_SETTLE);
        // The stale-demand cutoff would also emit a zero-rate setpoint
        // into the settle window; the stream restarts with the next
        // accepted demand instead.
        self.streaming = false;
        info!("gimbal neutral commanded");
    }

    /// Closes out a silent demand stream: one zero-rate setpoint when
    /// no demand has arrived within the cutoff, so a released stick or
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
            let frame = encode_gimbal_rate_setpoint(
                self.seq,
                0.0,
                0.0,
                self.target_system,
                self.target_component,
            );
            self.send(&frame);
            self.streaming = false;
            info!("gimbal demand stream cut off with a zero-rate setpoint");
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

    use super::Px4GimbalControl;

    fn control() -> (Px4GimbalControl, tokio::sync::mpsc::Receiver<Vec<u8>>) {
        let (tx, rx) = tokio::sync::mpsc::channel(64);
        let mut control = Px4GimbalControl::new(tx, 1, 1);
        control.use_manual_clock();
        (control, rx)
    }

    fn queued_kind(rx: &mut tokio::sync::mpsc::Receiver<Vec<u8>>) -> (u32, Option<u16>) {
        let buf = rx.try_recv().expect("queued frame");
        let msg_id = u32::from(buf[7]) | (u32::from(buf[8]) << 8) | (u32::from(buf[9]) << 16);
        let command = (msg_id == 76).then(|| u16::from_le_bytes([buf[38], buf[39]]));
        (msg_id, command)
    }

    #[test]
    fn first_demand_claims_before_streaming() {
        let (mut control, mut rx) = control();
        control.rate_demand(0.5, -0.25);
        assert_eq!(
            queued_kind(&mut rx),
            (76, Some(super::CMD_GIMBAL_CONFIGURE))
        );
        assert_eq!(queued_kind(&mut rx).0, 282);
        assert!(control.streaming());
    }

    #[test]
    fn claim_reasserts_only_after_its_period() {
        let (mut control, mut rx) = control();
        control.rate_demand(0.1, 0.0);
        assert_eq!(
            queued_kind(&mut rx),
            (76, Some(super::CMD_GIMBAL_CONFIGURE))
        );
        assert_eq!(queued_kind(&mut rx).0, 282);
        // Within the period: demands stream without a fresh claim.
        control.advance_clock(Duration::from_millis(400));
        control.rate_demand(0.2, 0.0);
        assert_eq!(queued_kind(&mut rx).0, 282);
        // Past the period: the claim re-heals before the next demand.
        control.advance_clock(Duration::from_millis(700));
        control.rate_demand(0.3, 0.0);
        assert_eq!(
            queued_kind(&mut rx),
            (76, Some(super::CMD_GIMBAL_CONFIGURE))
        );
        assert_eq!(queued_kind(&mut rx).0, 282);
    }

    #[test]
    fn stale_demand_stream_cuts_off_with_one_zero_rate() {
        let (mut control, mut rx) = control();
        control.rate_demand(1.0, 1.0);
        assert_eq!(
            queued_kind(&mut rx),
            (76, Some(super::CMD_GIMBAL_CONFIGURE))
        );
        assert_eq!(queued_kind(&mut rx).0, 282);
        // Fresh demands keep the stream open.
        control.advance_clock(Duration::from_millis(100));
        control.maintain();
        assert!(rx.try_recv().is_err(), "no cutoff while fresh");
        // Silence past the cutoff closes the stream with zero rates.
        control.advance_clock(Duration::from_millis(400));
        control.maintain();
        let buf = rx.try_recv().expect("cutoff frame");
        let msg_id = u32::from(buf[7]) | (u32::from(buf[8]) << 8) | (u32::from(buf[9]) << 16);
        assert_eq!(msg_id, 282);
        let pitch = f32::from_le_bytes([buf[34], buf[35], buf[36], buf[37]]);
        let yaw = f32::from_le_bytes([buf[38], buf[39], buf[40], buf[41]]);
        assert_eq!((pitch, yaw), (0.0, 0.0));
        assert!(!control.streaming());
        // The cutoff fires once, not forever.
        control.advance_clock(Duration::from_millis(400));
        control.maintain();
        assert!(rx.try_recv().is_err(), "cutoff is one-shot");
    }

    #[test]
    fn neutral_sends_the_pitchyaw_command() {
        let (mut control, mut rx) = control();
        control.neutral();
        assert_eq!(
            queued_kind(&mut rx),
            (76, Some(super::CMD_GIMBAL_CONFIGURE))
        );
        assert_eq!(queued_kind(&mut rx), (76, Some(super::CMD_GIMBAL_PITCHYAW)));
    }

    #[test]
    fn neutral_settle_window_holds_off_idle_keepalives() {
        let (mut control, mut rx) = control();
        control.neutral();
        assert_eq!(
            queued_kind(&mut rx),
            (76, Some(super::CMD_GIMBAL_CONFIGURE))
        );
        assert_eq!(queued_kind(&mut rx), (76, Some(super::CMD_GIMBAL_PITCHYAW)));
        // Idle keepalives inside the window are dropped: a zero-rate
        // setpoint would replace the angle target and cancel the recenter.
        control.rate_demand(0.0, 0.0);
        control.advance_clock(Duration::from_millis(400));
        control.rate_demand(0.0, 0.0);
        control.maintain();
        assert!(rx.try_recv().is_err(), "the settle window must stay quiet");
        // Past the window the keepalive stream resumes.
        control.advance_clock(Duration::from_millis(500));
        control.rate_demand(0.0, 0.0);
        assert_eq!(queued_kind(&mut rx).0, 282);
    }

    #[test]
    fn real_demand_breaks_through_the_settle_window() {
        let (mut control, mut rx) = control();
        control.neutral();
        assert_eq!(
            queued_kind(&mut rx),
            (76, Some(super::CMD_GIMBAL_CONFIGURE))
        );
        assert_eq!(queued_kind(&mut rx), (76, Some(super::CMD_GIMBAL_PITCHYAW)));
        control.advance_clock(Duration::from_millis(100));
        control.rate_demand(0.5, 0.0);
        let buf = rx.try_recv().expect("deliberate demand steers immediately");
        let msg_id = u32::from(buf[7]) | (u32::from(buf[8]) << 8) | (u32::from(buf[9]) << 16);
        assert_eq!(msg_id, 282);
    }
}
