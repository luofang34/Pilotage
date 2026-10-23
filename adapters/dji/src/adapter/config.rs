use std::io;
use std::path::PathBuf;
use std::time::Duration;

use crate::{GimbalAngles, SerialError};

/// The evidence used to permit bounded pitch commands.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum DjiControlMode {
    /// Require fresh continuous gimbal reports within -90 through 0 degrees.
    #[default]
    Observed,
    /// Use an initial horizontal target confirmed by the operator for this start.
    ///
    /// This mode tracks command targets within -90 through 0 degrees. It does
    /// not measure pose or verify the aircraft radio link. Start sends no movement.
    BenchInitialHorizontal,
}

/// One verified attachment and its raw byte capture.
#[derive(Debug, Clone)]
pub struct DjiConfig {
    /// The exact connection to open. No fallback or device selection occurs.
    pub attachment: super::attachment::DjiAttachment,
    /// A new JSON-lines file. An existing path is never overwritten.
    pub capture_path: PathBuf,
    /// The operator's explicit choice of command evidence.
    pub control_mode: DjiControlMode,
    /// Permit experimental discrete Takeoff and Land on the upper USB interface.
    /// This does not enable continuous flight control.
    pub experimental_flight: bool,
    /// Permit lease-bound BODY-FRD velocity through confirmed virtual-stick mode.
    /// Entry requires neutral input and fresh airborne GPS-attitude feedback.
    /// A stopped lease cannot restart; release must be confirmed before a new lease.
    /// Native flight actions require the virtual-stick exchange to finish first.
    pub experimental_virtual_stick: bool,
    /// Allow one explicit, bounded virtual-stick enable-and-disable diagnostic.
    /// This permits no velocity writes and starts no diagnostic automatically.
    pub experimental_virtual_stick_diagnostic: bool,
    /// The north reference the operator verified for the aircraft's reported
    /// heading. Set, the adapter publishes the reported attitude as an
    /// authorized attitude under that reference, which geographic guidance
    /// requires; unset, the attitude stays a display report.
    pub heading_reference: Option<pilotage_protocol::HeadingReference>,
    /// The frame the operator verified the gimbal's reported yaw is
    /// relative to. Set, the adapter publishes the gimbal attitude on the
    /// wire under that frame, which a client needs to place the camera's
    /// view on the map; unset, the gimbal angles stay a payload-device
    /// report with no frame.
    pub gimbal_yaw_frame: Option<pilotage_protocol::GimbalYawFrame>,
}

/// A failure to start a DJI connection.
#[derive(Debug, thiserror::Error)]
pub enum DjiAdapterError {
    /// The explicit terminal could not be opened.
    #[error(transparent)]
    Serial(#[from] SerialError),
    /// The native upper USB interface could not be opened.
    #[cfg(target_os = "macos")]
    #[error("cannot open DJI upper USB: {0}")]
    Usb(#[from] pilotage_dji_usb::UsbError),
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
    /// The experimental payload contract could not be constructed.
    #[error("invalid DJI payload contract: {0}")]
    Payload(#[from] turret_core::Error),
}

/// A latched failure. A new explicit start is required to recover.
#[derive(Debug, thiserror::Error)]
pub enum DjiFault {
    /// The attachment failed or disconnected.
    #[error("DJI transport {operation} failed: {source}")]
    Transport {
        /// The attempted operation.
        operation: &'static str,
        /// The attachment error.
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
    /// The mobile stream contained an invalid envelope.
    #[error("DJI mobile framing failed: {0}")]
    LogicLink(#[source] crate::logiclink::LogicLinkError),
    /// A finite video recovery request could not safely continue.
    #[error("DJI keyframe request failed: {0}")]
    Keyframe(#[source] turret_dji::KeyframeAbort),
    /// A partial command could not safely be completed.
    #[error("DJI partial command was invalidated; the connection is closed")]
    PartialCommand,
    /// Device-side lifecycle checks failed.
    #[error("DJI payload execution failed: {source}")]
    Execution {
        /// The failed lifecycle check.
        #[source]
        source: turret_core::Error,
    },
    /// A pitch target does not match this attachment's control profile.
    #[error("DJI control profile {operation} failed: {source}")]
    Profile {
        /// The attempted operation.
        operation: &'static str,
        /// The profile validation error.
        #[source]
        source: turret_dji::PitchTargetError,
    },
}

/// Observed transport state. It does not certify physical gimbal movement.
#[derive(Debug)]
pub struct DjiStatus<'a> {
    /// The selected command mode.
    pub control_mode: DjiControlMode,
    /// The bench target after a full recorded write, or the initial confirmed zero.
    /// This value is not a measured pose.
    pub commanded_pitch_tenths: Option<i16>,
    /// Whether this attachment refuses further ordinary control.
    pub control_locked: bool,
    /// A matching response to the one read-only RC version request arrived.
    pub rc_version_observed: bool,
    /// Last forward gimbal report, including the raw angles.
    pub latest_angles: Option<GimbalAngles>,
    /// Host receive age of the last forward report.
    pub pose_age: Option<Duration>,
    /// Consecutive forward reports within the freshness interval.
    pub consecutive_poses: u8,
    /// Actual bytes received from the attachment.
    pub received_bytes: u64,
    /// Actual bytes accepted by the transport for transmission.
    pub written_bytes: u64,
    /// Bytes remaining in the only pending packet.
    pub pending_bytes: usize,
    /// Encoded video bytes discarded by the bounded receive queue.
    pub dropped_video_bytes: u64,
    /// Envelopes retained in the raw capture but not decoded by this adapter.
    pub unknown_envelopes: u64,
    /// A latched connection failure, if present.
    pub fault: Option<&'a DjiFault>,
}
