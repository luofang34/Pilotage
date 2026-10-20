#[cfg(target_os = "macos")]
use std::io;
use std::path::PathBuf;

use super::DjiAdapterError;
use super::connection::Transport;
use crate::RawSerial;

/// An explicitly selected DJI connection, without automatic fallback.
#[derive(Debug, Clone)]
pub enum DjiAttachment {
    /// The lower USB CDC terminal.
    Serial {
        /// The exact terminal path to open.
        path: PathBuf,
    },
    /// An already enumerated upper iAP/logiclink device.
    #[cfg(target_os = "macos")]
    MacUsb {
        /// The current IOKit registry identity of the USB device.
        registry_id: u64,
    },
}

pub(super) fn open_blocking(
    attachment: &DjiAttachment,
) -> Result<Box<dyn Transport>, DjiAdapterError> {
    match attachment {
        DjiAttachment::Serial { path } => Ok(Box::new(RawSerial::open_blocking(path)?)),
        #[cfg(target_os = "macos")]
        DjiAttachment::MacUsb { registry_id } => Ok(Box::new(
            pilotage_dji_usb::MacUsb::open_blocking(*registry_id)?,
        )),
    }
}

#[cfg(target_os = "macos")]
impl Transport for pilotage_dji_usb::MacUsb {
    fn framing(&self) -> super::input::Framing {
        super::input::Framing::LogicLink
    }

    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        self.try_read(bytes)
    }

    fn write(&mut self, bytes: &[u8], send_before: std::time::Instant) -> io::Result<usize> {
        self.try_write_until(bytes, send_before)
    }

    fn close_blocking(self: Box<Self>) -> io::Result<()> {
        (*self).close_blocking().map_err(io::Error::other)
    }
}
