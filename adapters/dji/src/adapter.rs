//! A single-writer connection with a bounded gimbal command envelope.

mod airframe;
mod attachment;
mod battery;
mod bench;
mod capability;
mod config;
mod connection;
mod control;
mod execution;
mod flight;
mod input;
mod motion;
mod observation;
mod recording;
#[cfg(test)]
mod tests;
mod video;
mod virtual_stick;

use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use pilotage_adapter_api::{
    AdapterCapabilities, ApplyOutcome, LinkLossEnactError, LinkLossPolicy, SourceIncarnation,
    StepBudget, StepOutcome, TelemetryBatch, VehicleAdapter, VideoSource,
};
use pilotage_protocol::{ScopeId, ScopedControlFrame, VehicleId};
use pilotage_timing::SimTick;
use turret_dji::Mini3ControlProfile;

pub use attachment::DjiAttachment;
use connection::{Connection, Transport};
use control::ControlState;
pub use input::EncodedVideoChunk;
use observation::Observations;
pub use video::EncodedVideoStream;
pub use virtual_stick::{
    DjiVirtualStickDiagnosticError, DjiVirtualStickDiagnosticPhase,
    DjiVirtualStickDiagnosticReason, DjiVirtualStickDiagnosticStatus, DjiVirtualStickModeEvidence,
};

const GIMBAL_SCOPE: &str = "vehicle.gimbal";
const FLIGHT_SCOPE: &str = "vehicle.motion";
// This host policy bounds synthesized commands, not the hardware's maximum speed.
const HOST_PITCH_RATE_LIMIT_RPS: f32 = core::f32::consts::PI / 6.0;
const BENCH_PITCH_RATE_LIMIT_RPS: f32 = core::f32::consts::PI / 36.0;

pub use config::{DjiAdapterError, DjiConfig, DjiControlMode, DjiFault, DjiStatus};

/// A DJI Mini 3 connection with bounded gimbal and opt-in discrete flight control.
///
/// The caller drives I/O with `step` at 10 ms intervals. Discrete Takeoff and
/// Land require explicit configuration on the upper USB interface. Continuous
/// flight control requires a separate experimental opt-in.
/// The default mode requires valid reports. The explicit bench mode tracks
/// command targets only. Neither mode infers physical completion.
pub struct DjiAdapter {
    vehicle: VehicleId,
    origin: Instant,
    profile: Mini3ControlProfile,
    connection: Connection,
    observations: Observations,
    control: ControlState,
    video_subscribed: bool,
    experimental_flight: bool,
    experimental_virtual_stick: bool,
    experimental_virtual_stick_diagnostic: bool,
}

impl DjiAdapter {
    /// Open one explicit attachment and a new raw capture file.
    ///
    /// No background task or reconnect loop is started. The first `step`
    /// sends one read-only PC-to-RC version request.
    ///
    /// # Errors
    /// Returns an error if the attachment, capture, or identity cannot be opened.
    pub fn start_blocking(vehicle: VehicleId, config: DjiConfig) -> Result<Self, DjiAdapterError> {
        let mut identity = [0; 16];
        File::open("/dev/urandom")
            .and_then(|mut file| file.read_exact(&mut identity))
            .map_err(DjiAdapterError::Identity)?;
        let capture = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&config.capture_path)
            .map_err(|source| DjiAdapterError::Capture {
                path: config.capture_path.clone(),
                source,
            })?;
        let transport = attachment::open_blocking(&config.attachment)?;
        let mut adapter = Self::from_connection(
            vehicle,
            transport,
            Box::new(capture),
            config.capture_path,
            SourceIncarnation::new(identity),
            config.control_mode,
            config.experimental_flight,
        )?;
        adapter.experimental_virtual_stick = config.experimental_virtual_stick;
        adapter.experimental_virtual_stick_diagnostic =
            config.experimental_virtual_stick_diagnostic;
        Ok(adapter)
    }

    fn from_connection(
        vehicle: VehicleId,
        transport: Box<dyn Transport>,
        capture: Box<dyn Write + Send>,
        capture_path: PathBuf,
        incarnation: SourceIncarnation,
        control_mode: DjiControlMode,
        experimental_flight: bool,
    ) -> Result<Self, DjiAdapterError> {
        let rate = match control_mode {
            DjiControlMode::Observed => HOST_PITCH_RATE_LIMIT_RPS,
            DjiControlMode::BenchInitialHorizontal => BENCH_PITCH_RATE_LIMIT_RPS,
        };
        let profile =
            Mini3ControlProfile::new(-900, turret_core::AngularRate::radians_per_second(rate)?)?;
        let mut contract = profile.capabilities()?;
        let unavailable = (control_mode == DjiControlMode::Observed)
            .then_some(turret_core::UnavailableReason::MissingFeedback);
        capability::set_availability(&mut contract, unavailable);
        let origin = Instant::now();
        Ok(Self {
            vehicle,
            origin,
            profile,
            connection: Connection::new(
                transport,
                capture,
                capture_path,
                profile,
                contract,
                incarnation,
                origin,
            ),
            observations: Observations::new(incarnation, profile),
            control: ControlState::new(control_mode, profile),
            video_subscribed: false,
            experimental_flight: experimental_flight && control_mode == DjiControlMode::Observed,
            experimental_virtual_stick: false,
            experimental_virtual_stick_diagnostic: false,
        })
    }

    /// Return the current observations, without implying a working motion route.
    #[must_use]
    pub fn status_cached(&self) -> DjiStatus<'_> {
        DjiStatus {
            control_mode: self.control.mode(),
            commanded_pitch_tenths: self
                .control
                .bench
                .as_ref()
                .map(bench::BenchState::commanded),
            control_locked: self.control.link_loss || self.connection.fault.is_some(),
            rc_version_observed: self.connection.version_observed,
            latest_angles: self.observations.latest.map(|pose| pose.angles),
            pose_age: self.observations.age(self.origin.elapsed()),
            consecutive_poses: self.observations.consecutive,
            received_bytes: self.connection.received_bytes,
            written_bytes: self.connection.written_bytes,
            pending_bytes: self.connection.pending_bytes(),
            dropped_video_bytes: self.connection.input.dropped_video_bytes(),
            unknown_envelopes: self.connection.input.unknown_envelopes(),
            fault: self.connection.fault.as_ref(),
        }
    }

    fn step_with_time_blocking(&mut self, mut clock: impl FnMut() -> Duration) {
        let io_started = clock();
        self.read_available_blocking(io_started);
        // Recording can take time. Recheck age at the command write boundary.
        let now = clock();
        self.connection
            .drive_motion(&self.observations, now, &mut self.control.sequence);
        self.connection.drive_mode_session(&self.observations, now);
        let ready = self.pending_pitch_ready(now);
        if !ready {
            self.control.cancel_observed();
        }
        let had_pending = self.connection.pending_pitch().is_some();
        self.connection
            .write_pending_blocking(now, ready, &self.observations, &mut clock);
        let written = self.connection.take_written_pitch();
        let keyframe_now = clock();
        let model_until = if !self.connection.input_clear() {
            None
        } else if self.control.mode() == DjiControlMode::Observed {
            self.observations.model_deadline(keyframe_now)
        } else {
            Some(Duration::MAX)
        };
        self.connection.drive_keyframe_blocking(
            &mut self.control.sequence,
            keyframe_now,
            model_until,
            &mut clock,
        );
        let completed_at = clock();
        if let Some(bench) = &mut self.control.bench {
            bench.exclude_io(io_started, completed_at);
            if let Some((pitch, _identity)) = written {
                match bench.written(pitch, completed_at) {
                    Ok(()) => tracing::debug!(
                        elapsed_ns = completed_at.as_nanos(),
                        commanded_pitch_tenths = pitch.tenths(),
                        "DJI bench target fully written and recorded; physical completion is unverified"
                    ),
                    Err(source) => self.connection.fail(DjiFault::Profile {
                        operation: "record written pitch",
                        source,
                    }),
                }
            }
            if !ready || (had_pending && written.is_none()) {
                bench.stalled(completed_at, self.connection.pending_pitch().is_some());
            }
        }
        self.record_observed_write(written, had_pending);
        self.refresh_contract(completed_at);
    }

    fn read_available_blocking(&mut self, now: Duration) {
        let revision = self.observations.model_revision();
        self.observations.expire_model(now);
        self.connection
            .read_available_blocking(now, &mut self.observations);
        // A new report cannot restore commands queued under an expired or
        // replaced model, including loss and recovery in one read batch.
        if self.control.mode() == DjiControlMode::Observed
            && revision != self.observations.model_revision()
        {
            self.control.cancel_observed();
            self.connection.invalidate_model();
            self.connection.invalidate_flight(
                pilotage_adapter_api::FlightProcedureReason::SourceChanged,
                now,
            );
        }
    }

    fn record_observed_write(
        &mut self,
        written: Option<(crate::GimbalPitchTarget, turret_core::CommandIdentity)>,
        had_pending: bool,
    ) {
        if let Some((pitch, identity)) = written
            && let Some((expected, plan)) = self.control.observed_write
            && expected == identity
        {
            self.control.observed_write = None;
            if plan.target() == pitch
                && let Some(planner) = &mut self.control.observed
                && !planner.written(plan)
            {
                tracing::debug!("DJI completed write belongs to a cancelled pitch plan");
            }
        }
        if self.connection.fault.is_some()
            || (had_pending && written.is_none() && self.connection.pending_pitch().is_none())
        {
            self.control.cancel_observed();
        }
    }

    fn refresh_contract(&mut self, now: Duration) {
        use turret_core::UnavailableReason;
        // Authority recovery is enforced by the control latch. Publishing it as
        // a capability change would revoke the lease needed to clear that latch.
        let reason = if self.connection.fault.is_some() {
            Some(UnavailableReason::Transport)
        } else if self.control.mode() == DjiControlMode::Observed && !self.movement_ready(now) {
            Some(UnavailableReason::MissingFeedback)
        } else {
            None
        };
        let mut contract = *self.connection.execution.capabilities();
        capability::set_availability(&mut contract, reason);
        let feedback = self
            .observations
            .angle_feedback(now, self.connection.fault.is_none());
        contract.gimbal.axes.pitch.feedback.angle = feedback;
        contract.gimbal.axes.roll.feedback.angle = feedback;
        contract.gimbal.axes.yaw.feedback.angle = feedback;
        if contract == *self.connection.execution.capabilities() {
            return;
        }
        self.control.cancel_observed();
        let reason = if self.connection.cancel_movement() {
            reason
        } else {
            Some(UnavailableReason::Transport)
        };
        contract.revision = contract.revision.wrapping_add(1);
        capability::set_availability(&mut contract, reason);
        self.connection.execution.update_capabilities(contract);
    }

    fn tick(&self) -> SimTick {
        SimTick::new(self.origin.elapsed().as_nanos().min(u128::from(u64::MAX)) as u64)
    }
}

impl VehicleAdapter for DjiAdapter {
    fn capabilities(&self) -> AdapterCapabilities {
        let mut capabilities = capability::capabilities(
            self.vehicle,
            self.control.mode(),
            self.profile,
            self.connection.execution.capabilities(),
            self.observations.incarnation(),
            self.flight_enabled(),
            self.motion_enabled(),
        );
        capabilities.execution.render_capable = self.video_subscribed;
        capabilities
    }

    fn apply_control(&mut self, frame: &ScopedControlFrame) -> ApplyOutcome {
        self.apply_at(frame, self.origin.elapsed())
    }

    fn invalidate_pending_commands(&mut self, vehicle: VehicleId, scope: &ScopeId) {
        self.invalidate_scope(vehicle, scope, self.origin.elapsed());
    }

    fn sample_telemetry(&mut self) -> TelemetryBatch {
        let mut batch = self.observations.sample(
            self.vehicle,
            self.tick(),
            self.origin.elapsed(),
            self.connection.fault.is_none(),
        );
        let progress = self.connection.execution.wire_progress();
        let revision = self.connection.execution.capabilities().revision;
        if let Some(sample) = batch.samples.first_mut() {
            if sample.payload.is_none() && progress.is_some() {
                sample.payload = Some(self.observations.empty_state());
            }
            if let Some(payload) = sample.payload.as_mut() {
                payload.command = progress;
                payload.capability_revision = revision;
            }
        } else if progress.is_some() {
            let mut state = self.observations.empty_state();
            state.command = progress;
            state.capability_revision = revision;
            batch.samples.push(observation::telemetry(
                self.vehicle,
                self.tick(),
                Some(state),
            ));
        }
        self.attach_flight_progress(&mut batch);
        batch
    }

    fn video_sources(&self) -> Vec<VideoSource> {
        self.advertised_video_sources()
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
