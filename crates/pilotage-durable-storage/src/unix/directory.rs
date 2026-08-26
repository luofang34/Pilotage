use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;

use rustix::fs::{AtFlags, Dir, fchmod, fstat, mkdirat, statat};
use rustix::io::Errno;

use crate::{
    ContentDigest, DurabilityStep, ExactObject, ObjectInspection, ObjectName, OwnedTemporary,
    PrivateTreeLimits, PrivateTreeManifest, PutOutcome, StorageError, StorageOperation,
    StorageResult,
};

use super::anchor::{DirectoryHandle, open_directory};
use super::barrier::{sync_directory, syscall};
use super::metadata::{directory_mode, identity, inspect_private};
use super::objects;
use super::temporary;
use super::writer::WriterLease;

/// One verified directory in an anchored storage session.
#[derive(Clone)]
pub struct DurableDirectory {
    pub(crate) handle: DirectoryHandle,
}

impl DurableDirectory {
    pub(crate) const fn new(handle: DirectoryHandle) -> Self {
        Self { handle }
    }

    /// Create or open one exact private child directory.
    pub fn child(&self, lease: &WriterLease, name: &ObjectName) -> StorageResult<Self> {
        let context = self.handle.context(
            Some(name),
            StorageOperation::CreateDirectory,
            DurabilityStep::BeforeMutation,
        );
        lease.validate_for(&self.handle, &context)?;
        let existed = match statat(&self.handle.fd, name.as_os_str(), AtFlags::SYMLINK_NOFOLLOW) {
            Ok(_) => true,
            Err(Errno::NOENT) => false,
            Err(source) => {
                return Err(StorageError::Io {
                    context,
                    source: source.into(),
                });
            }
        };
        if !existed {
            self.create_child(lease, name)?;
        }
        let fd = open_directory(&self.handle.fd, name).map_err(|source| StorageError::Io {
            context: context.clone(),
            source: source.into(),
        })?;
        let stat = fstat(&fd).map_err(|source| StorageError::Io {
            context: context.clone(),
            source: source.into(),
        })?;
        let inspected = inspect_private(&stat, &context)?;
        let child = Self::new(self.handle.child(fd, name.clone(), inspected.identity));
        child.handle.validate(&context)?;
        let child_barrier = self.handle.context(
            Some(name),
            StorageOperation::CreateDirectory,
            DurabilityStep::ObjectData,
        );
        sync_directory(&child.handle.fd, &self.handle.anchor.faults, &child_barrier)?;
        let parent_barrier = self.handle.context(
            Some(name),
            StorageOperation::CreateDirectory,
            DurabilityStep::ParentDirectory,
        );
        sync_directory(&self.handle.fd, &self.handle.anchor.faults, &parent_barrier)?;
        let after = child.handle.context(
            Some(name),
            StorageOperation::CreateDirectory,
            DurabilityStep::AfterMutation,
        );
        lease.validate_for(&child.handle, &after)?;
        Ok(child)
    }

    /// Report whether one name identifies a valid managed object.
    pub fn exists(&self, name: &ObjectName) -> StorageResult<bool> {
        let context = self.handle.context(
            Some(name),
            StorageOperation::InspectObject,
            DurabilityStep::Selection,
        );
        self.handle.validate(&context)?;
        let result = match statat(&self.handle.fd, name.as_os_str(), AtFlags::SYMLINK_NOFOLLOW) {
            Ok(stat) => {
                inspect_private(&stat, &context)?;
                true
            }
            Err(Errno::NOENT) => false,
            Err(source) => {
                return Err(StorageError::Io {
                    context,
                    source: source.into(),
                });
            }
        };
        self.handle.validate(&context)?;
        Ok(result)
    }

    /// List normal components in byte order.
    pub fn list(&self) -> StorageResult<Vec<ObjectName>> {
        let context = self.handle.context(
            None,
            StorageOperation::ListDirectory,
            DurabilityStep::Selection,
        );
        self.handle.validate(&context)?;
        let directory = Dir::read_from(&self.handle.fd).map_err(|source| StorageError::Io {
            context: context.clone(),
            source: source.into(),
        })?;
        let mut names = Vec::new();
        for entry in directory {
            let entry = entry.map_err(|source| StorageError::Io {
                context: context.clone(),
                source: source.into(),
            })?;
            let bytes = entry.file_name().to_bytes();
            if bytes == b"." || bytes == b".." {
                continue;
            }
            let name = OsStr::from_bytes(bytes);
            names.push(
                ObjectName::new(name).map_err(|_| StorageError::invalid_name_at(name, &context))?,
            );
        }
        names.sort();
        self.handle.validate(&context)?;
        Ok(names)
    }

    /// Inspect one exact private object without following links.
    pub fn inspect(&self, name: &ObjectName) -> StorageResult<ObjectInspection> {
        objects::inspect(&self.handle, name)
    }

    /// Read a regular private object up to an exact byte limit.
    pub fn read_exact(
        &self,
        name: &ObjectName,
        maximum_bytes: usize,
    ) -> StorageResult<ExactObject> {
        objects::read_exact(&self.handle, name, maximum_bytes)
    }

    /// Read a private object and require one SHA-256 digest.
    pub fn read_digest(
        &self,
        name: &ObjectName,
        expected: ContentDigest,
        maximum_bytes: usize,
    ) -> StorageResult<ExactObject> {
        objects::read_digest(&self.handle, name, expected, maximum_bytes)
    }

    /// Publish an immutable object without replacing another object.
    pub fn put_immutable_no_replace(
        &self,
        lease: &WriterLease,
        name: &ObjectName,
        object: &ExactObject,
    ) -> StorageResult<PutOutcome> {
        objects::put_immutable(self, lease, name, object)
    }

    /// Repair and read one committed immutable publication.
    pub fn repair_immutable_publication_blocking(
        &self,
        lease: &WriterLease,
        name: &ObjectName,
        maximum_bytes: usize,
    ) -> StorageResult<Option<ExactObject>> {
        objects::repair_immutable_publication(self, lease, name, maximum_bytes)
    }

    /// Remove bounded uncommitted temporary objects.
    pub fn cleanup_unlinked_temporaries_blocking(
        &self,
        lease: &WriterLease,
        maximum_objects: usize,
        maximum_bytes: usize,
    ) -> StorageResult<usize> {
        temporary::cleanup_unlinked(self, lease, maximum_objects, maximum_bytes)
    }

    /// Inspect a private temporary object created by this store.
    pub fn inspect_owned_temporary(
        &self,
        name: &ObjectName,
        maximum_bytes: usize,
    ) -> StorageResult<OwnedTemporary> {
        temporary::inspect_owned(self, name, maximum_bytes)
    }

    /// Capture one exact private tree manifest without following links.
    pub fn inspect_private_tree(
        &self,
        name: &ObjectName,
        limits: PrivateTreeLimits,
    ) -> StorageResult<PrivateTreeManifest> {
        super::remove::inspect_private_tree(self, name, limits)
    }

    fn create_child(&self, lease: &WriterLease, name: &ObjectName) -> StorageResult<()> {
        let create = self.handle.context(
            Some(name),
            StorageOperation::CreateDirectory,
            DurabilityStep::Creation,
        );
        let creation = syscall(&self.handle.anchor.faults, &create, || {
            mkdirat(&self.handle.fd, name.as_os_str(), directory_mode())
        });
        let fd = match creation {
            Ok(()) => open_directory(&self.handle.fd, name),
            Err(original) => {
                match statat(&self.handle.fd, name.as_os_str(), AtFlags::SYMLINK_NOFOLLOW) {
                    Ok(stat) => {
                        inspect_private(&stat, &create)?;
                        open_directory(&self.handle.fd, name)
                    }
                    Err(Errno::NOENT) => return Err(original),
                    Err(source) => {
                        return Err(StorageError::Io {
                            context: create,
                            source: source.into(),
                        });
                    }
                }
            }
        }
        .map_err(|source| StorageError::Io {
            context: create.clone(),
            source: source.into(),
        })?;
        fchmod(&fd, directory_mode()).map_err(|source| StorageError::Io {
            context: create.clone(),
            source: source.into(),
        })?;
        let stat = fstat(&fd).map_err(|source| StorageError::Io {
            context: create.clone(),
            source: source.into(),
        })?;
        inspect_private(&stat, &create)?;
        let object = self.handle.context(
            Some(name),
            StorageOperation::CreateDirectory,
            DurabilityStep::ObjectData,
        );
        sync_directory(&fd, &self.handle.anchor.faults, &object)?;
        let parent = self.handle.context(
            Some(name),
            StorageOperation::CreateDirectory,
            DurabilityStep::ParentDirectory,
        );
        sync_directory(&self.handle.fd, &self.handle.anchor.faults, &parent)?;
        let after = self.handle.context(
            Some(name),
            StorageOperation::CreateDirectory,
            DurabilityStep::AfterMutation,
        );
        lease.validate_for(&self.handle, &after)?;
        let named = statat(&self.handle.fd, name.as_os_str(), AtFlags::SYMLINK_NOFOLLOW).map_err(
            |source| StorageError::Io {
                context: parent.clone(),
                source: source.into(),
            },
        )?;
        if identity(&named) != identity(&stat) {
            return Err(StorageError::IdentityChanged {
                expected: identity(&stat),
                actual: Some(identity(&named)),
                context: parent,
            });
        }
        Ok(())
    }
}
