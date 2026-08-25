use std::ffi::OsStr;
use std::io;
#[cfg(not(any(target_vendor = "apple", target_os = "linux", target_os = "android")))]
use std::path::Path;

use thiserror::Error;

#[cfg(any(target_vendor = "apple", target_os = "linux", target_os = "android"))]
use crate::{ContentDigest, DurabilityStep, StorageOperation};
use crate::{ObjectIdentity, RootIdentity, StorageContext};

/// A result from durable storage.
pub type StorageResult<T> = Result<T, StorageError>;

/// An error from a guarded compare-and-swap operation.
#[derive(Debug, Error)]
pub enum CompareExchangeError<E> {
    /// Durable storage rejected or could not complete the operation.
    #[error("{source}")]
    Storage {
        /// Durable-storage failure.
        #[source]
        source: StorageError,
    },
    /// The caller rejected authorization before the rename.
    #[error("compare-and-swap validation rejected authorization: {source}")]
    Validation {
        /// Caller validation failure.
        #[source]
        source: E,
    },
    /// Caller validation and temporary cleanup both failed.
    #[error(
        "compare-and-swap validation and temporary cleanup failed: validation={validation}; cleanup={cleanup}"
    )]
    ValidationAndCleanup {
        /// Caller validation failure.
        validation: E,
        /// Durable-storage cleanup failure.
        #[source]
        cleanup: StorageError,
    },
}

impl<E> CompareExchangeError<E> {
    pub(crate) const fn storage(source: StorageError) -> Self {
        Self::Storage { source }
    }
}

impl<E> From<StorageError> for CompareExchangeError<E> {
    fn from(source: StorageError) -> Self {
        Self::storage(source)
    }
}

/// A fail-closed durable-storage error.
#[derive(Debug, Error)]
pub enum StorageError {
    /// The target platform cannot supply the storage contract.
    #[error("durable storage is not supported on this platform: {context:?}")]
    UnsupportedPlatform {
        /// Available operation context.
        context: StorageContext,
    },
    /// A caller supplied more or less than one normal component.
    #[error("invalid object name {name:?}: {context:?}")]
    InvalidObjectName {
        /// Lossless operating-system name when available.
        name: Box<OsStr>,
        /// Available operation context.
        context: StorageContext,
    },
    /// An operating-system operation failed.
    #[error("storage I/O failed: {context:?}")]
    Io {
        /// Exact boundary that failed.
        context: StorageContext,
        /// Operating-system failure.
        #[source]
        source: io::Error,
    },
    /// A name identifies an unsupported object type.
    #[error("managed object has the wrong type: {context:?}")]
    WrongType {
        /// Exact boundary that failed.
        context: StorageContext,
    },
    /// A managed object has permissions other than the required exact mode.
    #[error("managed object mode {actual:#o} is not {required:#o}: {context:?}")]
    WrongMode {
        /// Required permission bits.
        required: u32,
        /// Observed permission bits.
        actual: u32,
        /// Exact boundary that failed.
        context: StorageContext,
    },
    /// A regular managed object has an unsafe link count.
    #[error("managed file link count {actual} is not one: {context:?}")]
    LinkedObject {
        /// Observed link count.
        actual: u64,
        /// Exact boundary that failed.
        context: StorageContext,
    },
    /// Exact object bytes do not match the required value.
    #[error("managed object bytes do not match: {context:?}")]
    ContentMismatch {
        /// Exact boundary that failed.
        context: StorageContext,
    },
    /// A file is larger than the caller's read limit.
    #[error("managed object size {actual} exceeds limit {limit}: {context:?}")]
    ObjectTooLarge {
        /// Caller limit in bytes.
        limit: usize,
        /// Observed size in bytes.
        actual: u64,
        /// Exact boundary that failed.
        context: StorageContext,
    },
    /// The root path no longer identifies the held root.
    #[error("storage root changed from {expected:?} to {actual:?}: {context:?}")]
    RootChanged {
        /// Held root identity.
        expected: RootIdentity,
        /// Current name identity, if the name exists.
        actual: Option<RootIdentity>,
        /// Exact boundary that failed.
        context: StorageContext,
    },
    /// A descendant name no longer identifies its held object.
    #[error("managed object changed from {expected:?} to {actual:?}: {context:?}")]
    IdentityChanged {
        /// Held object identity.
        expected: ObjectIdentity,
        /// Current name identity, if the name exists.
        actual: Option<ObjectIdentity>,
        /// Exact boundary that failed.
        context: StorageContext,
    },
    /// Another process holds the writer lease.
    #[error("another writer holds the storage lease: {context:?}")]
    WriterConflict {
        /// Exact boundary that failed.
        context: StorageContext,
    },
    /// The mutable object does not equal the caller's expected value.
    #[error("compare-and-swap expected value is stale: {context:?}")]
    StaleExpected {
        /// Exact boundary that failed.
        context: StorageContext,
    },
    /// Readback and the second barrier cannot resolve a commit result.
    #[error("storage commit result is ambiguous: {context:?}")]
    AmbiguousCommit {
        /// Exact recovery boundary that failed.
        context: StorageContext,
        /// The first failed durability boundary, when recovery also failed.
        prior: Option<Box<StorageError>>,
        /// Failure from the unresolved recovery boundary.
        #[source]
        source: Box<StorageError>,
    },
    /// A non-cooperating change or invalid managed relationship was found.
    #[error("managed storage is corrupt: {reason}: {context:?}")]
    Corruption {
        /// Stable description of the failed invariant.
        reason: &'static str,
        /// Exact boundary that failed.
        context: StorageContext,
    },
    /// A test fault fired at a real storage boundary.
    #[error("injected storage fault: {context:?}")]
    InjectedFault {
        /// Exact boundary where the fault fired.
        context: StorageContext,
    },
    /// The shared fault controller was poisoned.
    #[error("fault controller is poisoned: {context:?}")]
    FaultControllerPoisoned {
        /// Exact boundary requested by the caller.
        context: StorageContext,
    },
}

impl StorageError {
    /// Get the available structured context.
    #[must_use]
    pub const fn context(&self) -> &StorageContext {
        match self {
            Self::UnsupportedPlatform { context }
            | Self::InvalidObjectName { context, .. }
            | Self::Io { context, .. }
            | Self::WrongType { context }
            | Self::WrongMode { context, .. }
            | Self::LinkedObject { context, .. }
            | Self::ContentMismatch { context }
            | Self::ObjectTooLarge { context, .. }
            | Self::RootChanged { context, .. }
            | Self::IdentityChanged { context, .. }
            | Self::WriterConflict { context }
            | Self::StaleExpected { context }
            | Self::AmbiguousCommit { context, .. }
            | Self::Corruption { context, .. }
            | Self::InjectedFault { context }
            | Self::FaultControllerPoisoned { context } => context,
        }
    }

    /// Report whether authorization must stop after this error.
    #[must_use]
    pub const fn poisons_authorization(&self) -> bool {
        matches!(
            self,
            Self::AmbiguousCommit { .. }
                | Self::StaleExpected { .. }
                | Self::RootChanged { .. }
                | Self::IdentityChanged { .. }
                | Self::WrongType { .. }
                | Self::WrongMode { .. }
                | Self::LinkedObject { .. }
                | Self::ContentMismatch { .. }
                | Self::Corruption { .. }
        )
    }

    /// Report whether another process holds the writer lease.
    #[must_use]
    pub const fn is_writer_locked(&self) -> bool {
        matches!(self, Self::WriterConflict { .. })
    }

    pub(crate) fn invalid_name(name: &OsStr) -> Self {
        Self::InvalidObjectName {
            name: name.into(),
            context: StorageContext::root_open(),
        }
    }

    #[cfg(any(target_vendor = "apple", target_os = "linux", target_os = "android"))]
    pub(crate) fn invalid_name_at(name: &OsStr, context: &StorageContext) -> Self {
        Self::InvalidObjectName {
            name: name.into(),
            context: context.clone(),
        }
    }

    #[cfg(any(target_vendor = "apple", target_os = "linux", target_os = "android"))]
    pub(crate) fn at_object(
        mut self,
        operation: StorageOperation,
        object: ContentDigest,
        step: DurabilityStep,
    ) -> Self {
        let context = self.context_mut();
        context.operation = operation;
        context.object = Some(object);
        context.step = step;
        self
    }

    #[cfg(any(target_vendor = "apple", target_os = "linux", target_os = "android"))]
    fn context_mut(&mut self) -> &mut StorageContext {
        match self {
            Self::UnsupportedPlatform { context }
            | Self::InvalidObjectName { context, .. }
            | Self::Io { context, .. }
            | Self::WrongType { context }
            | Self::WrongMode { context, .. }
            | Self::LinkedObject { context, .. }
            | Self::ContentMismatch { context }
            | Self::ObjectTooLarge { context, .. }
            | Self::RootChanged { context, .. }
            | Self::IdentityChanged { context, .. }
            | Self::WriterConflict { context }
            | Self::StaleExpected { context }
            | Self::AmbiguousCommit { context, .. }
            | Self::Corruption { context, .. }
            | Self::InjectedFault { context }
            | Self::FaultControllerPoisoned { context } => context,
        }
    }

    #[cfg(not(any(target_vendor = "apple", target_os = "linux", target_os = "android")))]
    pub(crate) fn unsupported() -> Self {
        Self::UnsupportedPlatform {
            context: StorageContext::root_open(),
        }
    }

    #[cfg(not(any(target_vendor = "apple", target_os = "linux", target_os = "android")))]
    pub(crate) fn unsupported_at(path: &Path) -> Self {
        Self::UnsupportedPlatform {
            context: StorageContext::root_open_at(path),
        }
    }
}
