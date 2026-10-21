//! DJI command framing, bounded gimbal control, and encoded video reception.
//!
//! A parsed frame proves its checksums, not its origin or freshness.
//! The gimbal command encoder permits pitch targets from -90 to 0 degrees.
//! The adapter requires fresh reports for observed gimbal control.
//! An explicit bench mode tracks bounded command targets without pose.
//! Flight command responses remain separate from aircraft reports.
//! They do not establish an aircraft's ability to hover or stop after link loss.

#[cfg(any(target_os = "macos", target_os = "linux"))]
mod adapter;
pub mod flight;
pub mod logiclink;
#[cfg(any(target_os = "macos", target_os = "linux"))]
mod serial;

#[cfg(any(target_os = "macos", target_os = "linux"))]
pub use adapter::{
    DjiAdapter, DjiAdapterError, DjiAttachment, DjiConfig, DjiControlMode, DjiFault, DjiStatus,
    DjiVirtualStickDiagnosticError, DjiVirtualStickDiagnosticPhase,
    DjiVirtualStickDiagnosticReason, DjiVirtualStickDiagnosticStatus, DjiVirtualStickModeEvidence,
    EncodedVideoChunk, EncodedVideoStream,
};
#[cfg(any(target_os = "macos", target_os = "linux"))]
pub use serial::{RawSerial, SerialError};
pub use turret_dji::ParseStats;
pub use turret_dji::{DumlFrame, DumlParser, RouteAddress};
pub use turret_dji::{GimbalAngles, GimbalPitchTarget, PitchTargetError, pitch_response};
