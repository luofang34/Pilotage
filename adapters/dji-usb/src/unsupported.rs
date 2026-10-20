use std::io;

use crate::UsbError;

/// The unavailable macOS USB connection on another operating system.
pub struct MacUsb {
    _private: (),
}

impl MacUsb {
    /// Return an unsupported-platform error without accessing hardware.
    ///
    /// # Errors
    /// Always returns [`UsbError::UnsupportedPlatform`].
    pub fn open_blocking(_registry_id: u64) -> Result<Self, UsbError> {
        Err(UsbError::UnsupportedPlatform)
    }

    /// Return an unsupported-platform input error.
    ///
    /// # Errors
    /// Always returns [`io::ErrorKind::Unsupported`].
    pub fn try_read(&mut self, _output: &mut [u8]) -> io::Result<usize> {
        Err(io::ErrorKind::Unsupported.into())
    }

    /// Return an unsupported-platform output error.
    ///
    /// # Errors
    /// Always returns [`io::ErrorKind::Unsupported`].
    pub fn try_write_until(
        &mut self,
        _bytes: &[u8],
        _send_before: std::time::Instant,
    ) -> io::Result<usize> {
        Err(io::ErrorKind::Unsupported.into())
    }

    /// Return an unsupported-platform cleanup error.
    ///
    /// # Errors
    /// Always returns [`UsbError::UnsupportedPlatform`].
    pub fn close_blocking(self) -> Result<(), UsbError> {
        Err(UsbError::UnsupportedPlatform)
    }
}
