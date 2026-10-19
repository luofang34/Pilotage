//! DJI command framing and a bounded macOS gimbal adapter.
//!
//! A parsed frame proves its checksums, not its origin or freshness.
//! The gimbal command encoder permits pitch targets from -90 to 0 degrees.
//! The default adapter mode requires fresh reports and limits pitch to -5 to 0
//! degrees. An explicit bench mode tracks bounded command targets without pose.
//! It provides no flight control, flight estimate, or geographic position.

mod checksum;
mod duml;
mod gimbal;
mod stream;

#[cfg(target_os = "macos")]
mod adapter;
#[cfg(target_os = "macos")]
mod serial;

#[cfg(test)]
mod capture;

#[cfg(target_os = "macos")]
pub use adapter::{DjiAdapter, DjiAdapterError, DjiConfig, DjiControlMode, DjiFault, DjiStatus};
pub use duml::{DumlFrame, DumlParser, RouteAddress};
pub use gimbal::{GimbalAngles, GimbalPitchTarget, PitchTargetError, pitch_response};
#[cfg(target_os = "macos")]
pub use serial::{RawSerial, SerialError};
pub use stream::ParseStats;
