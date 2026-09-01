//! The trace path one non-nominal run is verified over.
//!
//! The launcher binds a loopback port before the executor starts, so the
//! endpoint the executor connects to is one the launch stated. The executor
//! opens the connection and states its run identities; the launcher accepts
//! them only when they are the identities it launched, and it acknowledges a
//! sample only after deriving every decision that sample states.
//!
//! An unacknowledged sample stops the executor, so a refusal here ends the
//! run rather than leaving a run that looks complete.

use std::io::ErrorKind;
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::time::Duration;

use flight_tune::{
    Digest, ExecutedLaunchIdentity, ExecutedStream, ExecutedUncertaintyReceipt, derivation,
};

use super::error::AviateConditionError;
use super::launch::ConditionLaunch;
use super::protocol::{
    TuningControlObservation, TuningFrameType, TuningHandshake, TuningHoverEstimatorMode,
    TuningObservationAck, TuningPerturbationCapability, TuningReady,
};
use super::{frame, projection};

const ACCEPT_TIMEOUT: Duration = Duration::from_secs(30);
const FRAME_TIMEOUT: Duration = Duration::from_secs(5);

/// One bound loopback trace path, before the executor starts.
#[derive(Debug)]
pub struct ConditionTracePath {
    listener: TcpListener,
    endpoint: SocketAddr,
}

impl ConditionTracePath {
    /// Binds one loopback trace path.
    ///
    /// The executor refuses an endpoint that is not on the loopback address,
    /// and the port is chosen by the operating system, so the launch states
    /// an endpoint no other run can hold.
    ///
    /// # Errors
    ///
    /// Returns [`AviateConditionError`] when the socket cannot be bound.
    pub fn bind_blocking() -> Result<Self, AviateConditionError> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .map_err(|source| AviateConditionError::trace("bind the trace path", source))?;
        let endpoint = listener
            .local_addr()
            .map_err(|source| AviateConditionError::trace("read the trace endpoint", source))?;
        Ok(Self { listener, endpoint })
    }

    /// Returns the endpoint the launch must state.
    #[must_use]
    pub const fn endpoint(&self) -> SocketAddr {
        self.endpoint
    }

    /// Verifies one complete run and seals what it executed.
    ///
    /// # Errors
    ///
    /// Returns [`AviateConditionError`] when the executor never connects,
    /// when it returns other identities, when the trace path fails, or when
    /// any sample does not state the decision the declaration required.
    pub fn verify_blocking(
        self,
        launch: &ConditionLaunch,
    ) -> Result<ExecutedUncertaintyReceipt, AviateConditionError> {
        let mut stream = self.accept_blocking()?;
        let handshake = frame::read::<TuningHandshake>(&mut stream)?;
        let returned = require_handshake(&handshake, launch)?;
        launch
            .identity()
            .require_returned(&returned)
            .map_err(|source| AviateConditionError::identity(source.to_string()))?;
        frame::write(
            &mut stream,
            &TuningReady {
                frame_type: TuningFrameType::AviateTuningReady,
                schema_version: launch.identity().trace_schema_version,
                run_manifest_digest: handshake.run_manifest_digest.clone(),
            },
        )?;
        self::verify_samples(&mut stream, launch, &handshake)
    }

    fn accept_blocking(&self) -> Result<TcpStream, AviateConditionError> {
        self.listener
            .set_nonblocking(false)
            .map_err(|source| AviateConditionError::trace("arm the trace path", source))?;
        let (stream, peer) = self
            .listener
            .accept()
            .map_err(|source| AviateConditionError::trace("accept the trace path", source))?;
        if !peer.ip().is_loopback() {
            return Err(AviateConditionError::protocol(
                "a trace connection arrived from outside the loopback address",
            ));
        }
        stream
            .set_read_timeout(Some(FRAME_TIMEOUT))
            .and_then(|()| stream.set_write_timeout(Some(FRAME_TIMEOUT)))
            .and_then(|()| stream.set_nodelay(true))
            .map_err(|source| AviateConditionError::trace("arm the trace connection", source))?;
        Ok(stream)
    }
}

/// Reads and verifies every sample until the executor closes the path.
fn verify_samples(
    stream: &mut TcpStream,
    launch: &ConditionLaunch,
    handshake: &TuningHandshake,
) -> Result<ExecutedUncertaintyReceipt, AviateConditionError> {
    let declaration = launch.declaration();
    let mut verified = ExecutedStream::open(declaration)
        .map_err(|source| AviateConditionError::Relation { source })?;
    let estimator_disabled = handshake.hover_estimator_mode != TuningHoverEstimatorMode::Online;
    loop {
        let observation = match frame::read::<TuningControlObservation>(stream) {
            Ok(observation) => observation,
            Err(AviateConditionError::Trace { source, .. })
                if source.kind() == ErrorKind::UnexpectedEof =>
            {
                break;
            }
            Err(error) => return Err(error),
        };
        let sample = projection::sample(
            &observation,
            launch.identity().trace_schema_version,
            estimator_disabled,
        )?;
        verified
            .accept(&sample)
            .map_err(|source| AviateConditionError::Relation { source })?;
        frame::write(
            stream,
            &TuningObservationAck {
                frame_type: TuningFrameType::AviateTuningObservationAck,
                schema_version: launch.identity().trace_schema_version,
                run_manifest_digest: handshake.run_manifest_digest.clone(),
                sequence: observation.sequence,
            },
        )?;
    }
    let summary = verified
        .close()
        .map_err(|source| AviateConditionError::Relation { source })?;
    ExecutedUncertaintyReceipt::new(
        launch.identity().clone(),
        declaration.clone(),
        summary.ledger,
        summary.sample_stream_digest,
    )
    .map_err(|source| AviateConditionError::Evidence { source })
}

/// Reads the identities one handshake returns.
///
/// The handshake must state the trace protocol, one complete condition
/// identity set, and a hover initialization the launcher can derive. A
/// handshake that omits any of these states nothing about the run.
fn require_handshake(
    handshake: &TuningHandshake,
    launch: &ConditionLaunch,
) -> Result<ExecutedLaunchIdentity, AviateConditionError> {
    let expected = launch.identity();
    if handshake.frame_type != TuningFrameType::AviateTuningHandshake
        || handshake.schema_version != expected.trace_schema_version
    {
        return Err(AviateConditionError::protocol(
            "the executor opened another trace protocol",
        ));
    }
    require_hover(handshake, launch)?;
    require_artifact_path(handshake, launch)?;
    let capabilities = handshake
        .condition_required_capabilities
        .as_ref()
        .ok_or_else(|| AviateConditionError::identity("the executor loaded no condition"))?;
    ExecutedLaunchIdentity::new(
        expected.run_intent_digest,
        require_digest(handshake.condition_artifact_sha256.as_deref(), "artifact")?,
        require_digest(handshake.condition_digest.as_deref(), "condition")?,
        handshake
            .condition_run_seed
            .ok_or_else(|| AviateConditionError::identity("the executor states no run seed"))?,
        capabilities.iter().copied().map(capability).collect(),
        handshake.schema_version,
    )
    .map_err(|source| AviateConditionError::identity(source.to_string()))
}

/// Requires the hover initialization the executor states to be derivable.
fn require_hover(
    handshake: &TuningHandshake,
    launch: &ConditionLaunch,
) -> Result<(), AviateConditionError> {
    if handshake.hover_estimator_mode == TuningHoverEstimatorMode::Online {
        return Err(AviateConditionError::identity(
            "the executor states an active online hover estimator",
        ));
    }
    if handshake.hover_kernel_config_hash != handshake.kernel_config_hash {
        return Err(AviateConditionError::identity(
            "the hover force is not carried by the resolved kernel",
        ));
    }
    if handshake.hover_scale_basis_points != launch.declaration().hover_scale_basis_points {
        return Err(AviateConditionError::identity(
            "the executor applied another hover force scale than the declared one",
        ));
    }
    let derived = derivation::scaled_hover_force(
        handshake.hover_baseline_force_bits,
        handshake.hover_scale_basis_points,
    );
    if derived != handshake.hover_effective_force_bits {
        return Err(AviateConditionError::identity(
            "the hover force does not follow from its baseline and scale",
        ));
    }
    Ok(())
}

/// Requires the executor to name the artifact this launch wrote.
fn require_artifact_path(
    handshake: &TuningHandshake,
    launch: &ConditionLaunch,
) -> Result<(), AviateConditionError> {
    let stated = handshake
        .condition_artifact_path
        .as_deref()
        .ok_or_else(|| {
            AviateConditionError::identity("the executor names no condition artifact")
        })?;
    if stated != launch.artifact_path().to_string_lossy() {
        return Err(AviateConditionError::identity(
            "the executor loaded another condition artifact",
        ));
    }
    Ok(())
}

const fn capability(value: TuningPerturbationCapability) -> flight_tune::BackendCapability {
    match value {
        TuningPerturbationCapability::ActuatorAuthority => {
            flight_tune::BackendCapability::ActuatorAuthority
        }
        TuningPerturbationCapability::CommandHold => flight_tune::BackendCapability::CommandHold,
        TuningPerturbationCapability::HoverTrimUncertainty => {
            flight_tune::BackendCapability::HoverTrimUncertainty
        }
        TuningPerturbationCapability::SensorPerturbation => {
            flight_tune::BackendCapability::SensorPerturbation
        }
    }
}

/// Reads one 64-character lowercase identity the executor states.
fn require_digest(
    stated: Option<&str>,
    name: &'static str,
) -> Result<Digest, AviateConditionError> {
    let stated = stated
        .ok_or_else(|| AviateConditionError::identity(format!("the executor states no {name}")))?;
    let mut bytes = [0_u8; 32];
    if stated.len() != 64 {
        return Err(AviateConditionError::identity(format!(
            "the executor {name} identity is not one digest"
        )));
    }
    for (index, byte) in bytes.iter_mut().enumerate() {
        let pair = stated
            .get(index * 2..index * 2 + 2)
            .ok_or_else(|| AviateConditionError::identity("an identity is not readable"))?;
        if pair
            .bytes()
            .any(|value| !value.is_ascii_lowercase() && !value.is_ascii_digit())
        {
            return Err(AviateConditionError::identity(format!(
                "the executor {name} identity is not lowercase"
            )));
        }
        *byte = u8::from_str_radix(pair, 16).map_err(|_| {
            AviateConditionError::identity(format!("the executor {name} identity is not a digest"))
        })?;
    }
    Ok(Digest::from_bytes(bytes))
}

/// The time the launcher waits for the executor to open the trace path.
#[must_use]
pub const fn accept_timeout() -> Duration {
    ACCEPT_TIMEOUT
}
