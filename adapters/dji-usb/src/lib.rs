//! Host-owned DJI upper USB access and a separate logiclink data owner.
//!
//! Read-only discovery reports candidate registry identities. This component
//! does not change USB roles. The opt-in access task handles the initial handshake.
//! Input buffering is bounded. Output success means an actual USB write completed.

mod access;
#[cfg(any(target_os = "macos", test))]
mod connection;
mod discovery;
pub use access::UsbAccessStatus;
mod error;
#[cfg(target_os = "macos")]
mod watch;
#[cfg(target_os = "macos")]
pub use watch::UsbEvents;
#[cfg(target_os = "macos")]
mod ffi;
#[cfg(all(not(target_os = "macos"), not(test)))]
mod unsupported;
#[cfg(any(target_os = "macos", test))]
mod worker;

#[cfg(any(target_os = "macos", test))]
pub use connection::MacUsb;
pub use discovery::{UpperUsbCandidate, discover_upper_blocking};
pub use error::UsbError;
#[cfg(all(not(target_os = "macos"), not(test)))]
pub use unsupported::MacUsb;
