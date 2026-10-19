//! A simulator's ground truth, received from the simulator itself rather
//! than through the flight controller (simulation builds only, ADR-0040).
//!
//! `PILOTAGE_SIM_TRUTH_ADDR` names a loopback UDP address. A simulator
//! (the JSBSim sidecar) sends one MAVLink `HIL_STATE_QUATERNION` per
//! physics step to it; the host decodes each and attaches the newest as
//! the vehicle's [`SimTruthSample`] when its adapter supplies none. A
//! truth that has stopped arriving is withheld, never replayed as live.

use std::net::UdpSocket;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use pilotage_adapter_api::{
    GeodeticFixSample, MeasurementClock, MeasurementStamp, SimTruthSample, SourceIncarnation,
    SourceIntegrity, SourceRole,
};
use pilotage_geo::{
    BaroSettingId, DatumRealizationId, GeodeticPosition, HorizontalDatum, LocalOriginId,
    PositionQuality, SIMULATOR_GEOID_MODEL_ID, TerrainRefId, VerticalDatum, VerticalPosition,
};
use pilotage_mavlink::FcMessage;

/// Truth older than this on the host's clock is withheld.
const STALE_AFTER: Duration = Duration::from_millis(500);
/// The truth lane's source id; one simulator feeds one host.
const SOURCE_ID: u64 = 1;
/// attitude | rates | position | velocity, as the estimate mask numbers them.
const TRUTH_VALID_FLAGS: u32 = 0x0f;

/// One `HIL_STATE_QUATERNION`, as decoded.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct TruthMessage {
    pub(super) time_usec: u64,
    pub(super) quat_wxyz: [f32; 4],
    pub(super) vel_ned_mps: [f32; 3],
    /// Latitude and longitude in degrees·1e7, altitude MSL in millimetres.
    pub(super) lat_lon_alt: [i32; 3],
}

/// The newest truth and the identity it rides.
#[derive(Debug)]
pub(super) struct TruthTracker {
    incarnation: SourceIncarnation,
    /// Bumped when simulation time steps back: the simulator reset.
    epoch: u32,
    sequence: u32,
    last_time_usec: Option<u64>,
    origin: Option<(f64, f64, f64)>,
    latest: Option<(SimTruthSample, Instant)>,
}

impl TruthTracker {
    pub(super) fn new(incarnation: SourceIncarnation) -> Self {
        Self {
            incarnation,
            epoch: 0,
            sequence: 0,
            last_time_usec: None,
            origin: None,
            latest: None,
        }
    }

    /// Takes one message received at `received`.
    pub(super) fn accept(&mut self, message: TruthMessage, received: Instant) {
        if self
            .last_time_usec
            .is_some_and(|last| message.time_usec < last)
        {
            // A reset simulator starts its clock again; samples before and
            // after are not one stream.
            self.epoch = self.epoch.wrapping_add(1);
            self.origin = None;
        }
        self.last_time_usec = Some(message.time_usec);
        self.sequence = self.sequence.wrapping_add(1);
        let [lat_e7, lon_e7, alt_mm] = message.lat_lon_alt;
        let latitude_deg = f64::from(lat_e7) * 1e-7;
        let longitude_deg = f64::from(lon_e7) * 1e-7;
        let altitude_m = f64::from(alt_mm) * 1e-3;
        let (lat0, lon0, alt0) =
            *self
                .origin
                .get_or_insert((latitude_deg, longitude_deg, altitude_m));
        let (per_lat, per_lon) = pilotage_world::metres_per_degree(lat0);
        let stamp = MeasurementStamp {
            role: SourceRole::SimulationTruth,
            // CRC-checked MAVLink on loopback, not authenticated.
            integrity: SourceIntegrity::ChecksummedOnly,
            source_id: SOURCE_ID,
            source_incarnation: self.incarnation,
            source_epoch: self.epoch,
            sequence: self.sequence,
            acquired_at_ns: message.time_usec.wrapping_mul(1_000),
            clock: MeasurementClock::Simulation,
        };
        let sample = SimTruthSample {
            quat_wxyz: message.quat_wxyz,
            pos_ned_m: [
                ((latitude_deg - lat0) * per_lat) as f32,
                ((longitude_deg - lon0) * per_lon) as f32,
                (alt0 - altitude_m) as f32,
            ],
            vel_ned_mps: message.vel_ned_mps,
            valid_flags: TRUTH_VALID_FLAGS,
            geodetic: geodetic(latitude_deg, longitude_deg, altitude_m, stamp),
            stamp,
        };
        self.latest = Some((sample, received));
    }

    /// The newest truth, unless it is older than [`STALE_AFTER`] at `now`.
    pub(super) fn latest(&self, now: Instant) -> Option<SimTruthSample> {
        self.latest
            .filter(|(_, received)| now.saturating_duration_since(*received) <= STALE_AFTER)
            .map(|(sample, _)| sample)
    }
}

/// The truth's position on WGS-84, MSL over the simulator's geoid.
fn geodetic(
    latitude_deg: f64,
    longitude_deg: f64,
    altitude_m: f64,
    stamp: MeasurementStamp,
) -> Option<GeodeticFixSample> {
    let vertical = VerticalPosition::new(
        altitude_m,
        VerticalDatum::Msl,
        SIMULATOR_GEOID_MODEL_ID,
        TerrainRefId::UNDECLARED,
        BaroSettingId::UNDECLARED,
        LocalOriginId::UNDECLARED,
    )
    .ok()?;
    let position = GeodeticPosition::new(
        latitude_deg,
        longitude_deg,
        HorizontalDatum::Wgs84,
        DatumRealizationId::UNDECLARED,
        vertical,
    )
    .ok()?;
    Some(GeodeticFixSample {
        position,
        // Truth: exact by construction, so no accuracy is stated.
        quality: PositionQuality {
            horizontal_mm: 0,
            vertical_mm: 0,
        },
        stamp,
    })
}

/// The lane: a socket and the tracker it feeds.
pub(super) struct SimTruthLane {
    socket: UdpSocket,
    tracker: TruthTracker,
    buffer: Vec<u8>,
    frames: Vec<(pilotage_mavlink::FrameSource, FcMessage)>,
}

impl SimTruthLane {
    /// Opens the lane when `PILOTAGE_SIM_TRUTH_ADDR` names an address.
    pub(super) fn from_env() -> Option<Self> {
        let addr = std::env::var("PILOTAGE_SIM_TRUTH_ADDR").ok()?;
        let socket = match UdpSocket::bind(&addr).and_then(|socket| {
            socket.set_nonblocking(true)?;
            Ok(socket)
        }) {
            Ok(socket) => socket,
            Err(error) => {
                tracing::warn!(%addr, %error, "simulator truth lane could not bind; no truth");
                return None;
            }
        };
        tracing::info!(%addr, "simulator truth lane listening");
        Some(Self {
            socket,
            tracker: TruthTracker::new(incarnation()),
            buffer: vec![0; 2048],
            frames: Vec::new(),
        })
    }

    /// Takes every datagram waiting, and returns the newest fresh truth.
    pub(super) fn poll(&mut self) -> Option<SimTruthSample> {
        let now = Instant::now();
        while let Ok(len) = self.socket.recv(&mut self.buffer) {
            self.frames.clear();
            pilotage_mavlink::parse_datagram(
                self.buffer.get(..len).unwrap_or_default(),
                &mut self.frames,
            );
            for (_, message) in &self.frames {
                if let FcMessage::SimTruth {
                    time_usec,
                    quat_wxyz,
                    vel_ned_mps,
                    lat_lon_alt,
                } = *message
                {
                    let message = TruthMessage {
                        time_usec,
                        quat_wxyz,
                        vel_ned_mps,
                        lat_lon_alt,
                    };
                    self.tracker.accept(message, now);
                }
            }
        }
        self.tracker.latest(now)
    }
}

/// An identity for this host's lane: the process and its start time, so
/// a restarted host is a new source.
fn incarnation() -> SourceIncarnation {
    let mut bytes = [0u8; 16];
    bytes[..4].copy_from_slice(&std::process::id().to_le_bytes());
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_nanos() as u64);
    bytes[4..12].copy_from_slice(&nanos.to_le_bytes());
    SourceIncarnation::new(bytes)
}

#[cfg(test)]
mod tests;
