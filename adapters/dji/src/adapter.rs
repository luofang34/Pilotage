//! A single-writer connection with a bounded gimbal command envelope.

mod capability;
mod connection;
mod control;
mod observation;
mod recording;
#[cfg(test)]
mod tests;

use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use pilotage_adapter_api::{
    AdapterCapabilities, ApplyOutcome, LinkLossEnactError, LinkLossPolicy, SourceIncarnation,
    StepBudget, StepOutcome, TelemetryBatch, VehicleAdapter, VideoSource,
};
use pilotage_protocol::{ScopeId, ScopedControlFrame, VehicleId};
use pilotage_timing::SimTick;

use crate::{GimbalAngles, RawSerial, SerialError};
use connection::{Connection, Transport};
use control::ControlState;
use observation::Observations;

const GIMBAL_SCOPE: &str = "vehicle.gimbal";

/// Explicit paths for one serial attachment and its raw byte capture.
#[derive(Debug, Clone)]
pub struct DjiConfig {
    /// A verified DJI CDC terminal. The adapter does not choose a device.
    pub serial_path: PathBuf,
    /// A new JSON-lines file. An existing path is never overwritten.
    pub capture_path: PathBuf,
}

/// A failure to start a DJI connection.
#[derive(Debug, thiserror::Error)]
pub enum DjiAdapterError {
    /// The explicit terminal could not be opened.
    #[error(transparent)]
    Serial(#[from] SerialError),
    /// The raw capture file could not be created.
    #[error("cannot create DJI capture {path:?}: {source}")]
    Capture {
        /// The requested capture path.
        path: PathBuf,
        /// The file system error.
        #[source]
        source: io::Error,
    },
    /// A fresh attachment identity could not be obtained.
    #[error("cannot obtain DJI attachment identity: {0}")]
    Identity(#[source] io::Error),
}

/// A latched failure. A new explicit start is required to recover.
#[derive(Debug, thiserror::Error)]
pub enum DjiFault {
    /// The terminal failed or disconnected.
    #[error("DJI serial {operation} failed: {source}")]
    Serial {
        /// The attempted operation.
        operation: &'static str,
        /// The terminal error.
        #[source]
        source: io::Error,
    },
    /// Recording failed, so further control is prohibited.
    #[error("DJI byte recording at {path:?} failed: {source}")]
    Recording {
        /// The capture file that failed.
        path: PathBuf,
        /// The recording error.
        #[source]
        source: io::Error,
    },
    /// The byte stream contained invalid or unknown framing.
    #[error("DJI byte stream lost validated DUML framing")]
    Framing,
    /// A partial command could not safely be completed.
    #[error("DJI partial command was invalidated; the connection is closed")]
    PartialCommand,
}

/// Observed transport state. It does not certify physical gimbal movement.
#[derive(Debug)]
pub struct DjiStatus<'a> {
    /// A matching response to the one read-only RC version request arrived.
    pub rc_version_observed: bool,
    /// Last forward gimbal report, including the raw angles.
    pub latest_angles: Option<GimbalAngles>,
    /// Host receive age of the last forward report.
    pub pose_age: Option<Duration>,
    /// Consecutive forward reports within the freshness interval.
    pub consecutive_poses: u8,
    /// Actual bytes received from the terminal.
    pub received_bytes: u64,
    /// Actual bytes accepted by the terminal for transmission.
    pub written_bytes: u64,
    /// Bytes remaining in the only pending packet.
    pub pending_bytes: usize,
    /// A latched connection failure, if present.
    pub fault: Option<&'a DjiFault>,
}

/// A DJI Mini 3 gimbal connection with no flight or motor command path.
///
/// The caller drives I/O with `step` at 10 ms intervals. Only a fixed RC
/// version request and bounded APP-to-gimbal pitch commands can be sent.
/// Valid reports are required, but they do not prove that this transport
/// can move the physical gimbal. Command completion is not inferred.
pub struct DjiAdapter {
    vehicle: VehicleId,
    origin: Instant,
    connection: Connection,
    observations: Observations,
    control: ControlState,
}

impl DjiAdapter {
    /// Open one explicit terminal and a new raw capture file.
    ///
    /// No background task or reconnect loop is started. The first `step`
    /// sends one read-only PC-to-RC version request.
    ///
    /// # Errors
    /// Returns an error if the terminal, capture, or identity cannot be opened.
    pub fn start_blocking(vehicle: VehicleId, config: DjiConfig) -> Result<Self, DjiAdapterError> {
        let mut identity = [0; 16];
        File::open("/dev/urandom")
            .and_then(|mut file| file.read_exact(&mut identity))
            .map_err(DjiAdapterError::Identity)?;
        let serial = RawSerial::open_blocking(&config.serial_path)?;
        let capture = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&config.capture_path)
            .map_err(|source| DjiAdapterError::Capture {
                path: config.capture_path.clone(),
                source,
            })?;
        Ok(Self::from_connection(
            vehicle,
            Box::new(serial),
            Box::new(capture),
            config.capture_path,
            SourceIncarnation::new(identity),
        ))
    }

    fn from_connection(
        vehicle: VehicleId,
        transport: Box<dyn Transport>,
        capture: Box<dyn Write + Send>,
        capture_path: PathBuf,
        incarnation: SourceIncarnation,
    ) -> Self {
        Self {
            vehicle,
            origin: Instant::now(),
            connection: Connection::new(transport, capture, capture_path),
            observations: Observations::new(incarnation),
            control: ControlState::default(),
        }
    }

    /// Return the current observations, without implying a working motion route.
    #[must_use]
    pub fn status_cached(&self) -> DjiStatus<'_> {
        DjiStatus {
            rc_version_observed: self.connection.version_observed,
            latest_angles: self.observations.latest.map(|pose| pose.angles),
            pose_age: self.observations.age(self.origin.elapsed()),
            consecutive_poses: self.observations.consecutive,
            received_bytes: self.connection.received_bytes,
            written_bytes: self.connection.written_bytes,
            pending_bytes: self.connection.pending_bytes(),
            fault: self.connection.fault.as_ref(),
        }
    }

    fn step_with_time_blocking(&mut self, mut clock: impl FnMut() -> Duration) {
        self.connection
            .read_available_blocking(clock(), &mut self.observations);
        // Recording can take time. Recheck age at the command write boundary.
        let now = clock();
        let ready = self.observations.ready(now) && self.connection.input_clear();
        self.connection.write_pending_blocking(now, ready);
    }

    fn tick(&self) -> SimTick {
        SimTick::new(self.origin.elapsed().as_nanos().min(u128::from(u64::MAX)) as u64)
    }
}

impl VehicleAdapter for DjiAdapter {
    fn capabilities(&self) -> AdapterCapabilities {
        capability::capabilities(self.vehicle)
    }

    fn apply_control(&mut self, frame: &ScopedControlFrame) -> ApplyOutcome {
        self.apply_at(frame, self.origin.elapsed())
    }

    fn sample_telemetry(&mut self) -> TelemetryBatch {
        self.observations.sample(
            self.vehicle,
            self.tick(),
            self.origin.elapsed(),
            self.connection.fault.is_none(),
        )
    }

    fn video_sources(&self) -> Vec<VideoSource> {
        Vec::new()
    }

    fn set_link_loss_policy(
        &mut self,
        vehicle: VehicleId,
        scope: &ScopeId,
        policy: Option<LinkLossPolicy>,
    ) -> Result<(), LinkLossEnactError> {
        let origin = self.origin;
        self.link_loss_with_time_blocking(vehicle, scope, policy, || origin.elapsed())
    }

    fn step(&mut self, _budget: StepBudget) -> StepOutcome {
        let origin = self.origin;
        self.step_with_time_blocking(|| origin.elapsed());
        StepOutcome {
            advanced: 0,
            now: self.tick(),
        }
    }
}
