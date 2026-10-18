//! `pilotage-multirotor-fdm`: runs the quad model in real time against a
//! Betaflight SITL process.
//!
//! Betaflight sends motor commands to UDP 9002 and reads state on UDP
//! 9003; both ports are fixed in its `sitl.c`. The model steps at a fixed
//! rate on the wall clock, takes the latest motor packet whenever one has
//! arrived, and sends a state packet every step. It never waits for a
//! motor packet: Betaflight skips a send when its update lock is busy, so
//! strict lockstep would stall.

use std::net::{SocketAddr, UdpSocket};
use std::time::{Duration, Instant};

use pilotage_multirotor_fdm::quad::GRAVITY;
use pilotage_multirotor_fdm::{
    FdmPacket, PacketClock, QuadParams, QuadState, Vec3, decode_servo_packet,
};

mod args;

use args::{Args, ArgsError};

/// Why the sidecar stopped.
#[derive(Debug, thiserror::Error)]
enum FdmError {
    #[error(transparent)]
    Args(#[from] ArgsError),
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

fn run_blocking(args: &Args) -> Result<(), FdmError> {
    let motors = UdpSocket::bind(args.motor_listen).map_err(|source| FdmError::Bind {
        addr: args.motor_listen,
        source,
    })?;
    motors
        .set_nonblocking(true)
        .map_err(|source| FdmError::Configure { source })?;
    let params = QuadParams::default();
    let mut state = QuadState::landed(Vec3::new(args.north_m, args.east_m, 0.0), args.heading_rad);
    let origin = args.origin;
    let dt = 1.0 / args.rate_hz;
    let step = Duration::from_secs_f64(dt);
    tracing::info!(
        listen = %args.motor_listen,
        fc = %args.state_target,
        rate_hz = args.rate_hz,
        hover_command = params.hover_command(),
        lat = origin.lat_deg,
        lon = origin.lon_deg,
        north_m = args.north_m,
        east_m = args.east_m,
        "multirotor FDM running"
    );
    let start = Instant::now();
    let mut clock = PacketClock::new(epoch_seconds(), args.rate_hz);
    let mut commands = [0.0; 4];
    let mut steps: u64 = 0;
    let mut last_log = start;
    let mut packets_in: u64 = 0;
    loop {
        packets_in = packets_in.wrapping_add(drain_motor_packets(&motors, &mut commands));
        let mut caught_up = 0;
        while sim_seconds(steps, dt) <= start.elapsed().as_secs_f64()
            && caught_up < MAX_CATCH_UP_STEPS
        {
            state.step(&params, commands, dt);
            steps = steps.wrapping_add(1);
            caught_up += 1;
        }
        let packet = FdmPacket::from_state(clock.tick(), &state, &origin);
        if let Err(error) = motors.send_to(&packet.to_bytes(), args.state_target) {
            tracing::debug!(%error, "state packet not sent");
        }
        if last_log.elapsed() >= LOG_PERIOD {
            last_log = Instant::now();
            log_truth(&state, &commands, packets_in);
        }
        let next = start + step.saturating_mul(u32::try_from(steps).unwrap_or(u32::MAX));
        if let Some(wait) = next.checked_duration_since(Instant::now()) {
            std::thread::sleep(wait);
        }
    }
}

/// Wall-clock seconds since the Unix epoch.
fn epoch_seconds() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0.0, |since| since.as_secs_f64())
}

fn sim_seconds(steps: u64, dt: f64) -> f64 {
    steps as f64 * dt
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
    tracing::info!(
        north = format_args!("{:.2}", state.position.x),
        east = format_args!("{:.2}", state.position.y),
        up = format_args!("{:.2}", -state.position.z),
        speed = format_args!("{:.2}", state.velocity.norm()),
        roll_deg = format_args!("{:.1}", roll.to_degrees()),
        pitch_deg = format_args!("{:.1}", pitch.to_degrees()),
        yaw_deg = format_args!("{:.1}", yaw.to_degrees()),
        load_g = format_args!("{:.2}", state.specific_force.norm() / GRAVITY),
        rates_dps = ?[state.rates.x.to_degrees().round(), state.rates.y.to_degrees().round(), state.rates.z.to_degrees().round()],
        motors = ?commands.map(|c| (c * 1000.0).round() / 1000.0),
        on_ground = state.on_ground,
        packets_in,
        "truth"
    );
}
