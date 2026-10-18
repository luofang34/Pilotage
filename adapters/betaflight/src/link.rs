//! The task that owns the MSP connection: it polls telemetry, runs the
//! outer loop, and streams RC at the poll rate.
//!
//! The adapter's trait methods are synchronous; they talk to this task
//! through two `watch` channels. The adapter writes the latest
//! [`LinkCommand`] (demand and arm requests) and reads the latest
//! [`LinkReport`]. Every poll cycle sends one `MSP_MULTIPLE_MSP` request
//! (status, attitude, IMU, altitude, GNSS in one round trip) and one
//! `MSP_SET_RAW_RC`, so Betaflight's receiver sees a frame every cycle and
//! its RX-loss failsafe stays quiet for exactly as long as this task runs.

use std::net::SocketAddr;
use std::time::{Duration, Instant};

use pilotage_adapter_api::FcCommandAck;
use pilotage_msp::{
    PayloadError, command, decode_multiple, encode_multiple_request, encode_set_raw_rc,
};
use tokio::net::TcpStream;
use tokio::sync::watch;

use crate::local_frame::LocalFrame;
use crate::outer_loop::{Envelope, Estimate, Horizontal};
use crate::rc::Sticks;
use crate::snapshot::{FcSnapshot, PollReplies};

mod arming;
mod pilot;
mod port;
mod setup;
#[cfg(test)]
mod tests;

pub(crate) use arming::Requests;
pub(crate) use pilot::{Demands, Family};

use pilot::Pilot;

use arming::ArmSwitch;
pub(crate) use setup::ConfigIssue;

use port::MspPort;

/// Poll and RC period: 50 Hz.
pub(crate) const POLL_PERIOD: Duration = Duration::from_millis(20);
/// Pause between connection attempts.
const RECONNECT_DELAY: Duration = Duration::from_millis(500);
/// The replies one poll batches, in reply order.
const POLLED: [u16; 5] = [
    command::MSP_STATUS_EX,
    command::MSP_ATTITUDE,
    command::MSP_RAW_IMU,
    command::MSP_ALTITUDE,
    command::MSP_RAW_GPS,
];

/// Why a connection ended.
#[derive(Debug, thiserror::Error)]
pub(crate) enum LinkError {
    #[error("MSP stream I/O failed while {during}: {source}")]
    Io {
        during: &'static str,
        #[source]
        source: std::io::Error,
    },
    #[error("the flight controller closed the MSP stream")]
    Closed,
    #[error("no reply to MSP command {command} within the reply timeout")]
    Timeout { command: u16 },
    #[error("the flight controller refused MSP command {command}")]
    Refused { command: u16 },
    #[error("the poll reply held {got} of {wanted} messages")]
    ShortPoll { got: usize, wanted: usize },
    #[error(transparent)]
    Payload(#[from] PayloadError),
    #[error(transparent)]
    Encode(#[from] pilotage_msp::EncodeError),
}

/// What the adapter asks of the link.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub(crate) struct LinkCommand {
    /// The latest demands of each family, and which family flies.
    pub(crate) demands: Demands,
    /// Arm, disarm, and refused-arm requests so far.
    pub(crate) requests: Requests,
}

/// What the link knows.
#[derive(Debug, Clone, PartialEq, Default)]
pub(crate) struct LinkReport {
    /// The latest poll, with its local position.
    pub(crate) latest: Option<Observation>,
    /// A configuration fact that blocks flight.
    pub(crate) issue: Option<ConfigIssue>,
    /// The FC's verdict on the latest arm or disarm request.
    pub(crate) last_command: Option<FcCommandAck>,
}

/// One poll plus what the adapter derived from it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Observation {
    /// The decoded poll.
    pub(crate) snapshot: FcSnapshot,
    /// North and east of the attachment's anchor, m, when the fix is
    /// usable.
    pub(crate) north_east: Option<[f64; 2]>,
    /// The anchor of the local frame.
    pub(crate) anchor: Option<[f64; 2]>,
    /// Whether the outer loop counts the quad as airborne.
    pub(crate) flying: bool,
    /// The outer loop's learned hover throttle, 0..1.
    pub(crate) hover_throttle: f64,
    /// The sticks this poll sent.
    pub(crate) sticks: Sticks,
}

/// Runs the link forever, reconnecting after every failure.
pub(crate) async fn run(
    endpoint: SocketAddr,
    envelope: Envelope,
    commands: watch::Receiver<LinkCommand>,
    reports: watch::Sender<LinkReport>,
) {
    let mut epoch: u32 = 0;
    let mut switch = ArmSwitch::default();
    loop {
        match TcpStream::connect(endpoint).await {
            Ok(stream) => {
                epoch = epoch.wrapping_add(1);
                stream.set_nodelay(true).ok();
                let mut session = Session::new(envelope, epoch, &commands, &reports);
                let ended = session.run(MspPort::new(stream), &mut switch).await;
                tracing::warn!(error = %ended, epoch, "Betaflight MSP link ended; reconnecting");
            }
            Err(error) => tracing::debug!(%error, %endpoint, "Betaflight MSP not reachable yet"),
        }
        reports.send_modify(|report| report.latest = None);
        tokio::time::sleep(RECONNECT_DELAY).await;
    }
}

/// One connection's state.
struct Session<'a> {
    epoch: u32,
    commands: watch::Receiver<LinkCommand>,
    reports: &'a watch::Sender<LinkReport>,
    pilot: Pilot,
    anchor: Option<LocalFrame>,
    anchor_deg: Option<[f64; 2]>,
    sequence: u32,
    last_step: Option<Instant>,
}

impl<'a> Session<'a> {
    fn new(
        envelope: Envelope,
        epoch: u32,
        commands: &'a watch::Receiver<LinkCommand>,
        reports: &'a watch::Sender<LinkReport>,
    ) -> Self {
        Self {
            epoch,
            commands: commands.clone(),
            reports,
            pilot: Pilot::new(envelope),
            anchor: None,
            anchor_deg: None,
            sequence: 0,
            last_step: None,
        }
    }

    async fn run(&mut self, mut port: MspPort<TcpStream>, switch: &mut ArmSwitch) -> LinkError {
        let setup = match setup::configure(&mut port).await {
            Ok(setup) => setup,
            Err(error) => return error,
        };
        if let Some(issue) = setup.issue {
            tracing::error!(%issue, "Betaflight configuration blocks flight");
        }
        self.reports
            .send_modify(|report| report.issue = setup.issue);
        let request = match encode_multiple_request(&POLLED) {
            Ok(request) => request,
            Err(error) => return error.into(),
        };
        match self.stream(&mut port, &request, &setup, switch).await {
            Ok(never) => match never {},
            Err(error) => error,
        }
    }

    /// The steady state. RC leaves on its own 50 Hz tick and never waits
    /// for a reply, so a slow telemetry reply (a loaded host, a slow serial
    /// task) delays the next poll but not the next RC frame: Betaflight's
    /// receiver sees a steady stream and its RX-loss timer stays quiet. A
    /// direct demand leaves the moment it arrives; the tick then only
    /// repeats it when no frame came.
    async fn stream(
        &mut self,
        port: &mut MspPort<TcpStream>,
        request: &[u8],
        setup: &setup::Setup,
        switch: &mut ArmSwitch,
    ) -> Result<std::convert::Infallible, LinkError> {
        let mut rc_tick = tokio::time::interval(POLL_PERIOD);
        let mut poll_tick = tokio::time::interval(POLL_PERIOD);
        poll_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut sticks = Sticks::IDLE;
        let mut polled_at: Option<Instant> = None;
        let mut sent_at: Option<Instant> = None;
        loop {
            let deadline =
                polled_at.map(|at| tokio::time::Instant::from_std(at + port::REPLY_TIMEOUT));
            tokio::select! {
                biased;
                changed = self.commands.changed() => {
                    changed.map_err(|_| LinkError::Closed)?;
                    let demands = self.commands.borrow_and_update().demands;
                    if let Some(next) = self.pilot.on_demands(&demands, Instant::now()) {
                        sticks = next;
                        port.send(command::MSP_SET_RAW_RC, encode_set_raw_rc(&sticks.channels())?).await?;
                        sent_at = Some(Instant::now());
                    }
                }
                _ = rc_tick.tick() => {
                    if sent_at.is_none_or(|at| at.elapsed() >= POLL_PERIOD / 2) {
                        port.send(command::MSP_SET_RAW_RC, encode_set_raw_rc(&sticks.channels())?).await?;
                        sent_at = Some(Instant::now());
                    }
                }
                frames = port.frames() => {
                    for frame in frames? {
                        let Some(reply) = port::reply_payload(frame, command::MSP_MULTIPLE_MSP) else {
                            continue;
                        };
                        let reply = reply?;
                        if let Some(at) = polled_at.take() && at.elapsed() > POLL_PERIOD * 2 {
                            tracing::debug!(reply_ms = at.elapsed().as_millis(), "slow Betaflight poll reply");
                        }
                        sticks = self.observe(&reply, setup, switch)?;
                    }
                }
                _ = poll_tick.tick(), if polled_at.is_none() => {
                    port.send(command::MSP_MULTIPLE_MSP, request.to_vec()).await?;
                    polled_at = Some(Instant::now());
                }
                () = sleep_until_or_never(deadline) => {
                    return Err(LinkError::Timeout { command: command::MSP_MULTIPLE_MSP });
                }
            }
        }
    }

    /// Folds one poll reply in and returns the sticks to send from now on.
    fn observe(
        &mut self,
        reply: &[u8],
        setup: &setup::Setup,
        switch: &mut ArmSwitch,
    ) -> Result<Sticks, LinkError> {
        let now = Instant::now();
        let snapshot = self.decode(reply, setup, now)?;
        let command = *self.commands.borrow();
        let blocked = setup.issue.is_some();
        let arm_high = switch.update(command.requests, snapshot.armed, now) && !blocked;
        let north_east = self.locate(&snapshot);
        let sticks = self.sticks(&snapshot, north_east, command, arm_high, now);
        let observation = Observation {
            snapshot,
            north_east,
            anchor: self.anchor_deg,
            flying: self.pilot.is_flying(),
            hover_throttle: self.pilot.hover_throttle(),
            sticks,
        };
        if self.sequence.is_multiple_of(50) {
            log_cycle(&observation);
        }
        self.reports.send_modify(|report| {
            report.latest = Some(observation);
            report.last_command = switch.last_command();
        });
        Ok(sticks)
    }

    fn decode(
        &mut self,
        reply: &[u8],
        setup: &setup::Setup,
        now: Instant,
    ) -> Result<FcSnapshot, LinkError> {
        let entries = decode_multiple(&POLLED, reply)?;
        let [status, attitude, imu, altitude, gps] = entries[..] else {
            return Err(LinkError::ShortPoll {
                got: entries.len(),
                wanted: POLLED.len(),
            });
        };
        self.sequence = self.sequence.wrapping_add(1);
        let replies = PollReplies {
            status: status.1,
            attitude: attitude.1,
            imu: imu.1,
            altitude: altitude.1,
            gps: gps.1,
        };
        Ok(FcSnapshot::decode(
            replies,
            &setup.boxes,
            now,
            self.sequence,
            self.epoch,
        )?)
    }

    /// The fix's north/east from the anchor, anchoring at the first fix.
    fn locate(&mut self, snapshot: &FcSnapshot) -> Option<[f64; 2]> {
        let fix = snapshot.gnss?;
        let frame = *self.anchor.get_or_insert_with(|| {
            tracing::info!(lat = fix.lat_deg, lon = fix.lon_deg, "local frame anchored");
            LocalFrame::anchored_at(fix.lat_deg, fix.lon_deg)
        });
        self.anchor_deg.get_or_insert([fix.lat_deg, fix.lon_deg]);
        Some(frame.north_east(fix.lat_deg, fix.lon_deg))
    }

    fn sticks(
        &mut self,
        snapshot: &FcSnapshot,
        north_east: Option<[f64; 2]>,
        command: LinkCommand,
        arm_high: bool,
        now: Instant,
    ) -> Sticks {
        let dt = self
            .last_step
            .replace(now)
            .map_or(POLL_PERIOD.as_secs_f64(), |last| (now - last).as_secs_f64());
        let estimate = Estimate {
            heading: snapshot.heading,
            roll: snapshot.roll,
            pitch: snapshot.pitch,
            horizontal: north_east
                .zip(snapshot.gnss)
                .map(|(position, fix)| Horizontal {
                    position,
                    velocity: fix.velocity_ne,
                }),
            altitude: snapshot.altitude_m,
            climb_rate: snapshot.climb_rate,
        };
        self.pilot.on_poll(
            &command.demands,
            &estimate,
            snapshot.armed,
            arm_high,
            now,
            dt,
        )
    }
}

/// A once-a-second trace of what the loop sees and sends.
fn log_cycle(observation: &Observation) {
    let snapshot = &observation.snapshot;
    tracing::debug!(
        armed = snapshot.armed,
        flying = observation.flying,
        blockers = ?snapshot.arming_disable.blockers(),
        north_east = ?observation.north_east.map(|ne| ne.map(|v| (v * 100.0).round() / 100.0)),
        altitude_m = snapshot.altitude_m,
        climb_rate = snapshot.climb_rate,
        heading_deg = format_args!("{:.1}", snapshot.heading.to_degrees()),
        roll_deg = format_args!("{:.1}", snapshot.roll.to_degrees()),
        pitch_deg = format_args!("{:.1}", snapshot.pitch.to_degrees()),
        rates_dps = ?snapshot.rates_frd.map(|r| r.to_degrees().round()),
        sticks = ?observation.sticks,
        hover = format_args!("{:.3}", observation.hover_throttle),
        "Betaflight link"
    );
}

/// Sleeps until `deadline`, or forever without one.
async fn sleep_until_or_never(deadline: Option<tokio::time::Instant>) {
    match deadline {
        Some(deadline) => tokio::time::sleep_until(deadline).await,
        None => std::future::pending().await,
    }
}
