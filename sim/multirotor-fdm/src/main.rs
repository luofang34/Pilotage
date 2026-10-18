//! `pilotage-multirotor-fdm`: runs the quad model in real time against a
//! Betaflight SITL process.
//!
//! Betaflight sends motor commands to UDP 9002 and reads state on UDP
//! 9003; both ports are fixed in its `sitl.c`. The model steps at a fixed
//! rate on the wall clock, takes the latest motor packet whenever one has
//! arrived, and sends a state packet every step. It never waits for a
//! motor packet: Betaflight skips a send when its update lock is busy, so
//! strict lockstep would stall.
//!
//! Given `--betaflight <elf>`, the sidecar also runs Betaflight as its
//! child: it loads the configuration, launches, and relaunches on the
//! failures `pilotage_multirotor_fdm::supervisor` describes, and streams
//! state only to a Betaflight that is ready for it. It prints
//! `Betaflight is flying the model` once motor commands flow.

use std::net::{SocketAddr, UdpSocket};
use std::time::{Duration, Instant};

use pilotage_multirotor_fdm::quad::GRAVITY;
use pilotage_multirotor_fdm::supervisor::{Action, Event, Supervisor};
use pilotage_multirotor_fdm::{
    FdmPacket, Origin, PacketClock, QuadParams, QuadState, Vec3, decode_servo_packet,
};

mod args;
mod sitl;

use args::{Args, ArgsError};
use sitl::{Sitl, SitlError};

/// Why the sidecar stopped.
#[derive(Debug, thiserror::Error)]
enum FdmError {
    #[error(transparent)]
    Args(#[from] ArgsError),
    #[error(transparent)]
    Sitl(#[from] SitlError),
    #[error("binding the motor-command socket {addr} failed: {source}")]
    Bind {
        addr: SocketAddr,
        #[source]
        source: std::io::Error,
    },
    #[error("configuring the motor-command socket failed: {source}")]
    Configure {
        #[source]
        source: std::io::Error,
    },
    #[error("gave up on Betaflight SITL: {reason}")]
    GaveUp { reason: &'static str },
}

/// The most physics steps run to catch up after one late wake-up; past
/// this the model drops wall time rather than spiral.
const MAX_CATCH_UP_STEPS: u32 = 50;
/// How often the true state is logged.
const LOG_PERIOD: Duration = Duration::from_secs(1);

fn main() -> Result<(), FdmError> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let args = Args::parse(&argv)?;
    run_blocking(&args)
}

/// The airframe and what it last heard and said.
struct Model {
    params: QuadParams,
    spawn: QuadState,
    state: QuadState,
    origin: Origin,
    commands: [f64; 4],
    clock: PacketClock,
}

impl Model {
    fn new(args: &Args) -> Self {
        let spawn = QuadState::landed(Vec3::new(args.north_m, args.east_m, 0.0), args.heading_rad);
        Self {
            params: QuadParams::default(),
            spawn,
            state: spawn,
            origin: args.origin,
            commands: [0.0; 4],
            clock: PacketClock::default(),
        }
    }

    /// Back on the ground at the spawn point, motors off: a new Betaflight
    /// starts from a fresh vehicle.
    fn reset(&mut self) {
        self.state = self.spawn;
        self.commands = [0.0; 4];
        self.clock = PacketClock::default();
    }
}

fn run_blocking(args: &Args) -> Result<(), FdmError> {
    let socket = UdpSocket::bind(args.motor_listen).map_err(|source| FdmError::Bind {
        addr: args.motor_listen,
        source,
    })?;
    socket
        .set_nonblocking(true)
        .map_err(|source| FdmError::Configure { source })?;
    let mut model = Model::new(args);
    let mut sitl = args.betaflight.clone().map(Sitl::new).transpose()?;
    let mut supervisor = sitl.as_ref().map(|sitl| Supervisor::new(sitl.provisions()));
    // Without a child to supervise, Betaflight is someone else's: stream
    // from the start.
    let mut streaming = sitl.is_none();
    tracing::info!(
        listen = %args.motor_listen,
        fc = %args.state_target,
        rate_hz = args.rate_hz,
        hover_command = model.params.hover_command(),
        lat = model.origin.lat_deg,
        lon = model.origin.lon_deg,
        north_m = args.north_m,
        east_m = args.east_m,
        "multirotor FDM running"
    );
    let dt = 1.0 / args.rate_hz;
    let step = Duration::from_secs_f64(dt);
    let start = Instant::now();
    let mut events = vec![Event::Start];
    let mut steps: u64 = 0;
    let mut last_log = start;
    let mut packets_in: u64 = 0;
    loop {
        let arrived = drain_motor_packets(&socket, &mut model.commands);
        packets_in = packets_in.wrapping_add(arrived);
        if let (Some(sitl), Some(supervisor)) = (sitl.as_mut(), supervisor.as_mut()) {
            if arrived > 0 {
                events.push(Event::MotorPackets(arrived));
            }
            events.extend(sitl.poll());
            events.push(Event::Tick);
            supervise(sitl, supervisor, &mut events, &mut model, &mut streaming)?;
        }
        let mut caught_up = 0;
        while steps as f64 * dt <= start.elapsed().as_secs_f64() && caught_up < MAX_CATCH_UP_STEPS {
            if streaming {
                model.state.step(&model.params, model.commands, dt);
            }
            steps = steps.wrapping_add(1);
            caught_up += 1;
        }
        if streaming {
            let packet = FdmPacket::from_state(model.clock.tick(), &model.state, &model.origin);
            if let Err(error) = socket.send_to(&packet.to_bytes(), args.state_target) {
                tracing::debug!(%error, "state packet not sent");
            }
        }
        if last_log.elapsed() >= LOG_PERIOD {
            last_log = Instant::now();
            log_truth(&model.state, &model.commands, packets_in);
        }
        let next = start + step.saturating_mul(u32::try_from(steps).unwrap_or(u32::MAX));
        if let Some(wait) = next.checked_duration_since(Instant::now()) {
            std::thread::sleep(wait);
        }
    }
}

/// Feeds the pending events to the supervisor and carries out what it
/// decides, until no event is left.
fn supervise(
    sitl: &mut Sitl,
    supervisor: &mut Supervisor,
    events: &mut Vec<Event>,
    model: &mut Model,
    streaming: &mut bool,
) -> Result<(), FdmError> {
    while !events.is_empty() {
        let event = events.remove(0);
        for action in supervisor.handle(event, Instant::now()) {
            match action {
                Action::Provision => events.push(Event::ProvisionFinished {
                    ok: sitl.provision_blocking()?,
                }),
                Action::Launch => sitl.launch()?,
                Action::StartStreaming => {
                    model.reset();
                    *streaming = true;
                }
                Action::StopStreaming => *streaming = false,
                Action::Kill { reason } => {
                    tracing::warn!(reason, "restarting Betaflight SITL");
                    sitl.kill();
                }
                Action::ReportFlying => tracing::info!("Betaflight is flying the model"),
                Action::GiveUp { reason } => return Err(FdmError::GaveUp { reason }),
            }
        }
    }
    Ok(())
}

/// Reads every queued motor packet, keeping the newest. Returns how many
/// arrived.
fn drain_motor_packets(socket: &UdpSocket, commands: &mut [f64; 4]) -> u64 {
    let mut buffer = [0u8; 64];
    let mut received = 0;
    while let Ok((len, _)) = socket.recv_from(&mut buffer) {
        if let Some(decoded) = decode_servo_packet(&buffer[..len]) {
            *commands = decoded;
            received += 1;
        }
    }
    received
}

fn log_truth(state: &QuadState, commands: &[f64; 4], packets_in: u64) {
    let (roll, pitch, yaw) = state.attitude.to_euler();
    let rates = [state.rates.x, state.rates.y, state.rates.z].map(|r| r.to_degrees().round());
    tracing::info!(
        north = format_args!("{:.2}", state.position.x),
        east = format_args!("{:.2}", state.position.y),
        up = format_args!("{:.2}", -state.position.z),
        speed = format_args!("{:.2}", state.velocity.norm()),
        roll_deg = format_args!("{:.1}", roll.to_degrees()),
        pitch_deg = format_args!("{:.1}", pitch.to_degrees()),
        yaw_deg = format_args!("{:.1}", yaw.to_degrees()),
        rates_dps = ?rates,
        load_g = format_args!("{:.2}", state.specific_force.norm() / GRAVITY),
        motors = ?commands.map(|c| (c * 1000.0).round() / 1000.0),
        on_ground = state.on_ground,
        packets_in,
        "truth"
    );
}
