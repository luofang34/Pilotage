use std::cell::Cell;
use std::sync::Arc;

use rustix::fd::OwnedFd;
use rustix::fs::{AtFlags, FlockOperation, Mode, OFlags, fchmod, flock, fstat, openat, statat};
use rustix::io::Errno;

use crate::{
    DurabilityStep, ObjectIdentity, ObjectName, StorageContext, StorageError, StorageOperation,
    StorageResult,
};

use super::anchor::{Anchor, DirectoryHandle};
use super::barrier::{sync_directory, sync_file};
use super::metadata::{file_mode, identity, inspect_private};

const WRITER_LOCK: &str = ".pilotage-writer-lock";

/// An owned nonblocking writer lease for one anchored root.
pub struct WriterLease {
    pub(crate) anchor: Arc<Anchor>,
    lock_fd: OwnedFd,
    lock_identity: ObjectIdentity,
    next_temporary: Cell<u64>,
}

impl WriterLease {
    pub(crate) fn acquire(anchor: Arc<Anchor>) -> StorageResult<Self> {
        let name = ObjectName::new(WRITER_LOCK)?;
        let context = anchor.context(
            Some(&name),
            StorageOperation::AcquireWriter,
            DurabilityStep::BeforeMutation,
        );
        anchor.validate_root(&context)?;
        let (lock_fd, created) = open_lock(&anchor, &name, &context)?;
        if created {
            fchmod(&lock_fd, file_mode()).map_err(|source| StorageError::Io {
                context: context.clone(),
                source: source.into(),
            })?;
        }
        let stat = fstat(&lock_fd).map_err(|source| StorageError::Io {
            context: context.clone(),
            source: source.into(),
        })?;
        let inspected = inspect_private(&stat, &context)?;
        let object = StorageContext {
            step: DurabilityStep::ObjectData,
            ..context.clone()
        };
        sync_file(&lock_fd, &anchor.faults, &object)?;
        let parent = StorageContext {
            step: DurabilityStep::ParentDirectory,
            ..context.clone()
        };
        sync_directory(&anchor.root_fd, &anchor.faults, &parent)?;
        match flock(&lock_fd, FlockOperation::NonBlockingLockExclusive) {
            Ok(()) => {}
            Err(error) if error == Errno::WOULDBLOCK || error == Errno::AGAIN => {
                return Err(StorageError::WriterConflict { context });
            }
            Err(source) => {
                return Err(StorageError::Io {
                    context,
                    source: source.into(),
                });
            }
        }
        let lease = Self {
            anchor,
            lock_fd,
            lock_identity: inspected.identity,
            next_temporary: Cell::new(0),
        };
        let root = DirectoryHandle::root(Arc::clone(&lease.anchor));
        let after = lease.anchor.context(
            Some(&name),
            StorageOperation::AcquireWriter,
            DurabilityStep::AfterMutation,
        );
        lease.validate_for(&root, &after)?;
        Ok(lease)
    }

    /// Validate the held lease and its anchored name binding.
    pub fn validate(&self, directory: &super::directory::DurableDirectory) -> StorageResult<()> {
        let name = ObjectName::new(WRITER_LOCK)?;
        let context = directory.handle.context(
            Some(&name),
            StorageOperation::ValidateWriter,
            DurabilityStep::Selection,
        );
        self.validate_for(&directory.handle, &context)
    }

    pub(crate) fn validate_for(
        &self,
        directory: &DirectoryHandle,
        context: &StorageContext,
    ) -> StorageResult<()> {
        if !directory.same_anchor(&self.anchor) {
            return Err(StorageError::Corruption {
                reason: "writer lease belongs to a different storage root",
                context: context.clone(),
            });
        }
        directory.validate(context)?;
        let name = ObjectName::new(WRITER_LOCK)?;
        let mut lock_context = context.clone();
        lock_context.component = Some(name.clone());
        let named = match statat(
            &self.anchor.root_fd,
            name.as_os_str(),
            AtFlags::SYMLINK_NOFOLLOW,
        ) {
            Ok(named) => named,
            Err(Errno::NOENT) => {
                return Err(StorageError::IdentityChanged {
                    expected: self.lock_identity,
                    actual: None,
                    context: lock_context,
                });
            }
            Err(source) => {
                return Err(StorageError::Io {
                    context: lock_context,
                    source: source.into(),
                });
            }
        };
        let inspected = inspect_private(&named, &lock_context)?;
        let held = fstat(&self.lock_fd).map_err(|source| StorageError::Io {
            context: lock_context.clone(),
            source: source.into(),
        })?;
        if inspected.identity != self.lock_identity || identity(&held) != self.lock_identity {
            return Err(StorageError::Corruption {
                reason: "writer lease name binding changed",
                context: lock_context,
            });
        }
        Ok(())
    }

    pub(crate) fn next_temporary_name(&self) -> StorageResult<ObjectName> {
        let current = self.next_temporary.get();
        self.next_temporary.set(current.wrapping_add(1));
        ObjectName::new(format!(
            ".pilotage-tmp-{}-{current:016x}",
            std::process::id()
        ))
    }

    pub(crate) fn is_writer_lock_name(name: &ObjectName) -> bool {
        name.as_os_str() == WRITER_LOCK
    }
}

fn open_lock(
    anchor: &Anchor,
    name: &ObjectName,
    context: &StorageContext,
) -> StorageResult<(OwnedFd, bool)> {
    let created = openat(
        &anchor.root_fd,
        name.as_os_str(),
        OFlags::RDWR
            | OFlags::CREATE
            | OFlags::EXCL
            | OFlags::NONBLOCK
            | OFlags::NOFOLLOW
            | OFlags::CLOEXEC,
        file_mode(),
    );
    match created {
        Ok(fd) => Ok((fd, true)),
        Err(Errno::EXIST) => {
            let fd = openat(
                &anchor.root_fd,
                name.as_os_str(),
                OFlags::RDWR | OFlags::NONBLOCK | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .map_err(|source| StorageError::Io {
                context: context.clone(),
                source: source.into(),
            })?;
            Ok((fd, false))
        }
        Err(source) => Err(StorageError::Io {
            context: context.clone(),
            source: source.into(),
        }),
    }
}
