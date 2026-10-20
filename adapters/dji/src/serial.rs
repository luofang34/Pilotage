//! Nonblocking terminal I/O with terminal settings restoration.

use std::io;
use std::os::fd::OwnedFd;
use std::path::{Path, PathBuf};

use rustix::fs::{FlockOperation, Mode, OFlags, flock, open};
use rustix::termios::{
    OptionalActions, Termios, ioctl_tiocexcl, ioctl_tiocnxcl, tcgetattr, tcsetattr,
};
use thiserror::Error;
use tracing::warn;

/// A terminal operation failed.
#[derive(Debug, Error)]
pub enum SerialError {
    /// The operating system rejected a terminal operation.
    #[error("cannot {operation} terminal {}: {source}", path.display())]
    Operation {
        /// The operation that failed.
        operation: &'static str,
        /// The requested terminal path.
        path: PathBuf,
        /// The operating system error.
        #[source]
        source: io::Error,
    },
}

impl SerialError {
    /// Return the I/O error kind, including `WouldBlock` and `Interrupted`.
    pub fn kind(&self) -> io::ErrorKind {
        match self {
            Self::Operation { source, .. } => source.kind(),
        }
    }
}

/// An exclusive terminal descriptor with no internal command queue.
///
/// The caller must verify the device identity before it opens the path.
/// Opening and closing a terminal can change hardware modem signals.
/// This type does not send modem-control ioctls or change the baud rate.
pub struct RawSerial {
    fd: OwnedFd,
    path: PathBuf,
    original: Option<Termios>,
    exclusive: bool,
}

impl RawSerial {
    /// Open an explicit terminal path in nonblocking raw mode.
    ///
    /// Symbolic links are rejected. Pending input is not discarded.
    /// The original baud rate and terminal settings are saved for close.
    /// It requests terminal exclusivity and takes an advisory lock.
    /// Existing readers are not removed.
    pub fn open_blocking(path: &Path) -> Result<Self, SerialError> {
        let flags =
            OFlags::RDWR | OFlags::NOCTTY | OFlags::NONBLOCK | OFlags::CLOEXEC | OFlags::NOFOLLOW;
        let fd =
            open(path, flags, Mode::empty()).map_err(|source| Self::error(path, "open", source))?;
        flock(&fd, FlockOperation::NonBlockingLockExclusive)
            .map_err(|source| Self::error(path, "lock", source))?;
        let mut serial = Self {
            fd,
            path: path.to_owned(),
            original: None,
            exclusive: false,
        };
        ioctl_tiocexcl(&serial.fd)
            .map_err(|source| Self::error(path, "set exclusive access on", source))?;
        serial.exclusive = true;
        let original = tcgetattr(&serial.fd)
            .map_err(|source| Self::error(path, "read settings from", source))?;
        let mut raw = original.clone();
        serial.original = Some(original);
        raw.make_raw();
        tcsetattr(&serial.fd, OptionalActions::Now, &raw)
            .map_err(|source| Self::error(path, "set raw mode on", source))?;
        Ok(serial)
    }

    /// Read available bytes without waiting for input.
    ///
    /// An unavailable read returns an error with kind `WouldBlock`.
    /// Pass a nonempty buffer to distinguish the end of input.
    pub fn try_read(&mut self, bytes: &mut [u8]) -> Result<usize, SerialError> {
        rustix::io::read(&self.fd, bytes)
            .map_err(|source| Self::error(&self.path, "read from", source))
    }

    /// Write as many bytes as the terminal can accept without waiting.
    ///
    /// A successful call can write fewer bytes than requested. The caller
    /// owns the unwritten bytes and decides whether they are still valid.
    pub fn try_write(&mut self, bytes: &[u8]) -> Result<usize, SerialError> {
        rustix::io::write(&self.fd, bytes)
            .map_err(|source| Self::error(&self.path, "write to", source))
    }

    /// Restore terminal settings and release exclusive access before close.
    ///
    /// Both cleanup operations are attempted. If restoration fails, drop
    /// retries it on this descriptor and records any remaining failure.
    pub fn close_blocking(mut self) -> Result<(), SerialError> {
        self.restore_blocking()
    }

    fn restore_blocking(&mut self) -> Result<(), SerialError> {
        let restore = if let Some(original) = &self.original {
            tcsetattr(&self.fd, OptionalActions::Now, original)
                .map_err(|source| Self::error(&self.path, "restore settings on", source))
        } else {
            Ok(())
        };
        if restore.is_ok() {
            self.original = None;
        }
        let release = if self.exclusive {
            ioctl_tiocnxcl(&self.fd)
                .map_err(|source| Self::error(&self.path, "release exclusive access on", source))
        } else {
            Ok(())
        };
        if release.is_ok() {
            self.exclusive = false;
        }
        match (restore, release) {
            (Err(restore), Err(release)) => {
                warn!(error = %release, "terminal cleanup also failed");
                Err(restore)
            }
            (Err(error), _) | (_, Err(error)) => Err(error),
            (Ok(()), Ok(())) => Ok(()),
        }
    }

    fn error(path: &Path, operation: &'static str, source: rustix::io::Errno) -> SerialError {
        SerialError::Operation {
            operation,
            path: path.to_owned(),
            source: source.into(),
        }
    }
}

impl Drop for RawSerial {
    fn drop(&mut self) {
        if let Err(error) = self.restore_blocking() {
            warn!(%error, "terminal cleanup failed before close");
        }
    }
}

#[cfg(test)]
mod tests;
