//! DJI command framing and a bounded Unix gimbal adapter.
//!
//! A parsed frame proves its checksums, not its origin or freshness.
//! The gimbal command encoder permits pitch targets from -90 to 0 degrees.
//! The default adapter mode requires fresh reports and limits pitch to -5 to 0
//! degrees. An explicit bench mode tracks bounded command targets without pose.
//! It provides no flight control, flight estimate, or geographic position.

#[cfg(any(target_os = "macos", target_os = "linux"))]
mod adapter;
#[cfg(any(target_os = "macos", target_os = "linux"))]
mod serial;

#[cfg(any(target_os = "macos", target_os = "linux"))]
pub use adapter::{DjiAdapter, DjiAdapterError, DjiConfig, DjiControlMode, DjiFault, DjiStatus};
#[cfg(any(target_os = "macos", target_os = "linux"))]
pub use serial::{RawSerial, SerialError};
pub use turret_dji::ParseStats;
pub use turret_dji::{DumlFrame, DumlParser, RouteAddress};
pub use turret_dji::{GimbalAngles, GimbalPitchTarget, PitchTargetError, pitch_response};
