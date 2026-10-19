//! PX4's side of the link: listen where PX4's `simulator_mavlink`
//! connects, then step in lockstep with it.
//!
//! PX4 SITL runs on the simulator's clock. It cannot advance until a
//! `HIL_SENSOR` arrives, and once it has started sending
//! `HIL_ACTUATOR_CONTROLS` it answers every sensor step with one. The
//! session therefore sends a step's sensors, then waits for PX4's
//! answer before the next step; before PX4's first answer it steps
//! freely so PX4 can boot. Wall-clock pacing keeps simulation time near
//! real time when PX4 is fast enough.

use std::io::{ErrorKind, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::time::{Duration, Instant};

use pilotage_world::{TerrainSource, World};

use crate::aircraft::Aircraft;
use crate::engine::ModelPaths;
use crate::gimbal::GimbalLimits;
use crate::hil::{ActuatorControls, Encoder, FrameReader};
use crate::pacer::Pacer;
use crate::sensors::NoiseLevels;
use crate::simulation::{Simulation, SimulationError, Start};

/// How a session runs.
#[derive(Debug, Clone)]
pub struct SessionConfig {
    /// The loopback TCP port PX4 connects to (4560 + PX4 instance).
    pub port: u16,
    /// The aircraft.
    pub aircraft: Aircraft,
    /// Where its model lives.
    pub paths: ModelPaths,
    /// Where it starts.
    pub start: Start,
    /// The sensor noise seed.
    pub seed: u64,
    /// The sensor noise.
    pub noise: NoiseLevels,
    /// The gimbal's travel, when the vehicle carries one.
    pub gimbal: Option<GimbalLimits>,
    /// Simulation seconds per wall second; 0 runs unpaced.
    pub realtime_factor: f64,
    /// How long to wait for PX4's answer to a step before stepping on.
    pub lockstep_timeout: Duration,
}

/// Why the session stopped.
#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    /// The listening socket could not be opened.
    #[error("listening on 127.0.0.1:{port} failed: {source}")]
    Listen {
        /// The port.
        port: u16,
        /// The OS error.
        #[source]
        source: std::io::Error,
    },
    /// The simulation could not start.
    #[error("the simulation could not start: {0}")]
    Simulation(#[from] SimulationError),
}

/// The line the session logs once it listens; a launcher waits for it.
pub const LISTENING: &str = "jsbsim sidecar listening for PX4";

/// Serves PX4 connections one after another, each with a fresh vehicle
/// at the start, until the listener fails.
///
/// # Errors
///
/// Returns [`SessionError`] when the port cannot be bound or a vehicle
/// cannot be built.
pub fn serve_blocking<T: TerrainSource>(
    config: &SessionConfig,
    world: &mut World<T>,
) -> Result<(), SessionError> {
    let listener =
        TcpListener::bind(("127.0.0.1", config.port)).map_err(|source| SessionError::Listen {
            port: config.port,
            source,
        })?;
    tracing::info!(port = config.port, aircraft = ?config.aircraft, "{LISTENING}");
    let mut connection: u64 = 0;
    loop {
        // A vehicle is built before PX4 connects, so a model error shows
        // at start-up rather than at PX4's first step.
        let simulation = Simulation::new(
            config.aircraft,
            &config.paths,
            world,
            config.start,
            config.seed,
            config.noise,
            config.gimbal,
        )?;
        let truth = simulation.truth();
        tracing::info!(
            latitude_deg = truth.latitude_deg,
            longitude_deg = truth.longitude_deg,
            altitude_m = truth.altitude_m,
            heading_deg = truth.attitude.yaw.to_degrees(),
            "vehicle on the ground; waiting for PX4"
        );
        let stream = match listener.accept() {
            Ok((stream, peer)) => {
                connection = connection.wrapping_add(1);
                tracing::info!(%peer, connection, "PX4 connected");
                stream
            }
            Err(error) => {
                tracing::warn!(%error, "accepting PX4 failed");
                continue;
            }
        };
        match run_connection(config, simulation, stream, world) {
            Ok(()) => tracing::info!(connection, "PX4 disconnected; resetting the vehicle"),
            Err(error) => tracing::error!(%error, connection, "the connection ended; resetting"),
        }
    }
}

/// Why one connection ended.
#[derive(Debug, thiserror::Error)]
enum ConnectionError {
    #[error(transparent)]
    Simulation(#[from] SimulationError),
    #[error("the PX4 link failed: {0}")]
    Io(#[from] std::io::Error),
}

fn run_connection<T: TerrainSource>(
    config: &SessionConfig,
    mut simulation: Simulation,
    mut stream: TcpStream,
    world: &mut World<T>,
) -> Result<(), ConnectionError> {
    stream.set_nodelay(true)?;
    let mut encoder = Encoder::default();
    let mut reader = FrameReader::default();
    let mut controls: Option<ActuatorControls> = None;
    let mut lockstep = false;
    let mut pacer = Pacer::new(config.realtime_factor, Instant::now(), 0.0);
    let mut buffer = [0u8; 4096];
    let mut report_at = 0.0;
    let mut last_report = (Instant::now(), 0.0);
    loop {
        for frame in simulation.step(controls.as_ref(), world, &mut encoder)? {
            stream.write_all(&frame)?;
        }
        let wait = if lockstep {
            config.lockstep_timeout
        } else {
            Duration::from_millis(1)
        };
        match receive(&mut stream, &mut reader, &mut buffer, wait)? {
            Received::Controls(latest) => {
                lockstep |= latest.flags & 1 != 0;
                controls = Some(latest);
            }
            Received::Nothing if lockstep => {
                tracing::warn!(timeout = ?config.lockstep_timeout, "PX4 did not answer a step");
            }
            Received::Nothing => {}
            Received::Closed => return Ok(()),
        }
        let truth = simulation.truth();
        let armed = controls.is_some_and(|c| c.armed());
        // Once a second while armed, so a log alone retraces a flight from
        // its first metre; every ten seconds on the ground.
        let period_s = if armed { 1.0 } else { 10.0 };
        if truth.time_s >= report_at || report_at - truth.time_s > period_s {
            report_at = truth.time_s + period_s;
            tracing::info!(
                sim_time_s = truth.time_s,
                latitude_deg = truth.latitude_deg,
                longitude_deg = truth.longitude_deg,
                altitude_m = truth.altitude_m,
                heading_deg = truth.attitude.yaw.to_degrees().rem_euclid(360.0),
                ground_speed_mps = truth.velocity_ned[0].hypot(truth.velocity_ned[1]),
                armed,
                gimbal_deg = ?simulation.gimbal_angles_deg().map(|a| a.map(|d| d.round())),
                realtime_factor = (truth.time_s - last_report.1)
                    / last_report.0.elapsed().as_secs_f64().max(1e-3),
                "stepping"
            );
            last_report = (Instant::now(), truth.time_s);
        }
        if let Some(sleep) = pacer.wait(truth.time_s, Instant::now()) {
            std::thread::sleep(sleep);
        }
    }
}

enum Received {
    Controls(ActuatorControls),
    Nothing,
    Closed,
}

/// Reads until a `HIL_ACTUATOR_CONTROLS` arrives or `wait` passes, and
/// keeps only the newest one buffered.
fn receive(
    stream: &mut TcpStream,
    reader: &mut FrameReader,
    buffer: &mut [u8],
    wait: Duration,
) -> std::io::Result<Received> {
    let deadline = Instant::now() + wait;
    loop {
        if let Some(mut latest) = reader.next_actuator_controls() {
            while let Some(newer) = reader.next_actuator_controls() {
                latest = newer;
            }
            return Ok(Received::Controls(latest));
        }
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Ok(Received::Nothing);
        }
        stream.set_read_timeout(Some(left))?;
        match stream.read(buffer) {
            Ok(0) => return Ok(Received::Closed),
            Ok(n) => reader.push(buffer.get(..n).unwrap_or_default()),
            Err(error) if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {
                return Ok(Received::Nothing);
            }
            Err(error) if error.kind() == ErrorKind::ConnectionReset => {
                return Ok(Received::Closed);
            }
            Err(error) => return Err(error),
        }
    }
}
