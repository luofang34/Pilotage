//! Private crash-durable storage with anchored Unix directory handles.
//!
//! All cooperating writers must hold the anchored writer lease. A process with
//! the same user access can write without that lease. The store stops when it
//! detects such a change. POSIX does not compare file bytes and rename a file
//! in one operation. A same-user process can change a name after the last
//! validation. The operating system does not make validation and return one
//! operation. Isolate the storage root from non-cooperating same-user code.

#![allow(clippy::result_large_err)]

mod error;
#[cfg(any(
    test,
    feature = "fault-injection",
    target_vendor = "apple",
    target_os = "linux",
    target_os = "android"
))]
mod fault;
mod types;

#[cfg(not(any(target_vendor = "apple", target_os = "linux", target_os = "android")))]
mod non_unix;
#[cfg(any(target_vendor = "apple", target_os = "linux", target_os = "android"))]
mod unix;

pub use error::{CompareExchangeError, StorageError, StorageResult};
#[cfg(any(test, feature = "fault-injection"))]
pub use fault::{FaultAction, FaultController, FaultRule};
#[cfg(not(any(target_vendor = "apple", target_os = "linux", target_os = "android")))]
pub use non_unix::{DurableDirectory, DurableStore, WriterLease};
pub use types::{
    CasOutcome, ContentDigest, DurabilityStep, ExactObject, ExpectedValue, ObjectIdentity,
    ObjectInspection, ObjectKind, ObjectName, OwnedTemporary, PrivateTreeLimits,
    PrivateTreeManifest, PutOutcome, RootIdentity, StorageContext, StorageOperation, digest_bytes,
};
#[cfg(any(target_vendor = "apple", target_os = "linux", target_os = "android"))]
pub use unix::{DurableDirectory, DurableStore, WriterLease};

#[cfg(all(
    test,
    any(target_vendor = "apple", target_os = "linux", target_os = "android")
))]
mod tests;

#[cfg(all(
    test,
    not(any(target_vendor = "apple", target_os = "linux", target_os = "android"))
))]
mod non_unix_tests;
