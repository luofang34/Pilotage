use std::io;

/// A failure to establish or release exclusive logiclink ownership.
#[derive(Debug, thiserror::Error)]
pub enum UsbError {
    /// The current operating system has no implementation.
    #[error("DJI USB requires macOS")]
    UnsupportedPlatform,
    /// A registry identity must name one live attachment.
    #[error("DJI USB registry identity must be nonzero")]
    InvalidIdentity,
    /// The worker could not be created.
    #[error("cannot create DJI USB owner: {0}")]
    WorkerSpawn(#[source] io::Error),
    /// Startup did not report a definite result within its budget.
    #[error("DJI USB startup outcome is unknown")]
    StartupUnknown,
    /// Cleanup did not report a definite result within its budget.
    #[error("DJI USB cleanup outcome is unknown")]
    CleanupUnknown,
    /// The owner failed before it could confirm cleanup.
    #[error("DJI USB owner ended without confirming cleanup")]
    WorkerEnded,
    /// An IOKit operation failed.
    #[error("DJI USB native stage {stage} failed with status {status:#010x}")]
    Native {
        /// The native operation stage.
        stage: u32,
        /// The IOKit result or validation refusal.
        status: u32,
    },
}

#[derive(Clone, Debug)]
#[cfg(any(target_os = "macos", test))]
pub(crate) struct Failure {
    pub(crate) kind: io::ErrorKind,
    cause: FailureCause,
}

#[cfg(any(target_os = "macos", test))]
#[derive(Clone, Debug)]
enum FailureCause {
    Context(String),
    Native { stage: u32, status: u32 },
}

#[cfg(any(target_os = "macos", test))]
impl Failure {
    pub(crate) fn new(kind: io::ErrorKind, detail: impl Into<String>) -> Self {
        Self {
            kind,
            cause: FailureCause::Context(detail.into()),
        }
    }

    pub(crate) fn native(stage: u32, status: u32) -> Self {
        Self {
            kind: io::ErrorKind::Other,
            cause: FailureCause::Native { stage, status },
        }
    }

    pub(crate) fn io(&self) -> io::Error {
        match self.cause {
            FailureCause::Native { stage, status } => {
                io::Error::new(self.kind, UsbError::Native { stage, status })
            }
            FailureCause::Context(_) => io::Error::new(self.kind, self.clone()),
        }
    }
}

#[cfg(any(target_os = "macos", test))]
impl std::fmt::Display for Failure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.cause {
            FailureCause::Context(detail) => formatter.write_str(detail),
            FailureCause::Native { stage, status } => {
                write!(
                    formatter,
                    "DJI USB native stage {stage} failed with status {status:#010x}"
                )
            }
        }
    }
}

#[cfg(any(target_os = "macos", test))]
impl std::error::Error for Failure {}

#[cfg(test)]
mod tests;
