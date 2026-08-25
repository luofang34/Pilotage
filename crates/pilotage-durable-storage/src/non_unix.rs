use std::path::Path;

use crate::{
    CasOutcome, CompareExchangeError, ContentDigest, ExactObject, ExpectedValue, ObjectInspection,
    ObjectName, OwnedTemporary, PrivateTreeLimits, PrivateTreeManifest, PutOutcome, RootIdentity,
    StorageError, StorageResult,
};

/// A storage root that refuses use on an unsupported platform.
#[derive(Clone, Debug)]
pub struct DurableStore {
    _private: (),
}

/// A storage directory that refuses use on an unsupported platform.
#[derive(Clone, Debug)]
pub struct DurableDirectory {
    _private: (),
}

/// A writer lease that cannot exist on an unsupported platform.
#[derive(Debug)]
pub struct WriterLease {
    _private: (),
}

impl DurableStore {
    /// Refuse a weaker storage implementation.
    pub fn open_or_create(path: &Path) -> StorageResult<Self> {
        Err(StorageError::unsupported_at(path))
    }

    /// Refuse a fault-enabled weaker storage implementation.
    #[cfg(any(test, feature = "fault-injection"))]
    pub fn open_or_create_with_faults(
        path: &Path,
        _faults: crate::FaultController,
    ) -> StorageResult<Self> {
        Err(StorageError::unsupported_at(path))
    }

    /// Return the unavailable root identity.
    #[must_use]
    pub const fn root_identity(&self) -> RootIdentity {
        RootIdentity {
            device: 0,
            inode: 0,
        }
    }

    /// Return the unavailable root directory.
    #[must_use]
    pub const fn root_directory(&self) -> DurableDirectory {
        DurableDirectory { _private: () }
    }

    /// Refuse a writer lease on an unsupported platform.
    pub fn acquire_writer(&self) -> StorageResult<WriterLease> {
        Err(StorageError::unsupported())
    }
}

impl DurableDirectory {
    /// Refuse child access on an unsupported platform.
    pub fn child(&self, _lease: &WriterLease, _name: &ObjectName) -> StorageResult<Self> {
        Err(StorageError::unsupported())
    }

    /// Refuse object lookup on an unsupported platform.
    pub fn exists(&self, _name: &ObjectName) -> StorageResult<bool> {
        Err(StorageError::unsupported())
    }

    /// Refuse directory listing on an unsupported platform.
    pub fn list(&self) -> StorageResult<Vec<ObjectName>> {
        Err(StorageError::unsupported())
    }

    /// Refuse object inspection on an unsupported platform.
    pub fn inspect(&self, _name: &ObjectName) -> StorageResult<ObjectInspection> {
        Err(StorageError::unsupported())
    }

    /// Refuse exact reads on an unsupported platform.
    pub fn read_exact(
        &self,
        _name: &ObjectName,
        _maximum_bytes: usize,
    ) -> StorageResult<ExactObject> {
        Err(StorageError::unsupported())
    }

    /// Refuse digest-bound reads on an unsupported platform.
    pub fn read_digest(
        &self,
        _name: &ObjectName,
        _expected: ContentDigest,
        _maximum_bytes: usize,
    ) -> StorageResult<ExactObject> {
        Err(StorageError::unsupported())
    }

    /// Refuse immutable publication on an unsupported platform.
    pub fn put_immutable_no_replace(
        &self,
        _lease: &WriterLease,
        _name: &ObjectName,
        _object: &ExactObject,
    ) -> StorageResult<PutOutcome> {
        Err(StorageError::unsupported())
    }

    /// Refuse temporary inspection on an unsupported platform.
    pub fn inspect_owned_temporary(
        &self,
        _name: &ObjectName,
        _maximum_bytes: usize,
    ) -> StorageResult<OwnedTemporary> {
        Err(StorageError::unsupported())
    }

    /// Refuse private-tree inspection on an unsupported platform.
    pub fn inspect_private_tree(
        &self,
        _name: &ObjectName,
        _limits: PrivateTreeLimits,
    ) -> StorageResult<PrivateTreeManifest> {
        Err(StorageError::unsupported())
    }
}

impl WriterLease {
    /// Refuse writer-lease validation on an unsupported platform.
    pub fn validate(&self, _directory: &DurableDirectory) -> StorageResult<()> {
        Err(StorageError::unsupported())
    }

    /// Refuse compare-and-swap on an unsupported platform.
    pub fn compare_exchange_file(
        &self,
        _directory: &DurableDirectory,
        _name: &ObjectName,
        _expected: ExpectedValue,
        _new: ExactObject,
    ) -> StorageResult<CasOutcome> {
        Err(StorageError::unsupported())
    }

    /// Refuse guarded compare-and-swap on an unsupported platform.
    pub fn compare_exchange_file_guarded<E>(
        &self,
        _directory: &DurableDirectory,
        _name: &ObjectName,
        _expected: ExpectedValue,
        _new: ExactObject,
        _validate: impl FnOnce() -> Result<(), E>,
    ) -> Result<CasOutcome, CompareExchangeError<E>> {
        Err(CompareExchangeError::Storage {
            source: StorageError::unsupported(),
        })
    }

    /// Refuse exact tree removal on an unsupported platform.
    pub fn remove_private_tree(
        &self,
        _directory: &DurableDirectory,
        _manifest: &PrivateTreeManifest,
    ) -> StorageResult<()> {
        Err(StorageError::unsupported())
    }

    /// Refuse owned temporary cleanup on an unsupported platform.
    pub fn cleanup_owned_temporary(
        &self,
        _directory: &DurableDirectory,
        _temporary: &OwnedTemporary,
    ) -> StorageResult<()> {
        Err(StorageError::unsupported())
    }
}
