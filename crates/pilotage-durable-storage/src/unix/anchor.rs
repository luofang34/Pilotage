use std::path::PathBuf;
use std::sync::Arc;

use rustix::fd::OwnedFd;
use rustix::fs::{AtFlags, OFlags, fstat, openat, statat};
use rustix::io::Errno;

use crate::fault::FaultController;
use crate::{
    DurabilityStep, ObjectIdentity, ObjectName, RootIdentity, StorageContext, StorageError,
    StorageOperation, StorageResult,
};

use super::metadata::{PRIVATE_DIRECTORY_MODE, identity, inspect_private};

pub(crate) struct Anchor {
    pub(crate) root_parent: OwnedFd,
    pub(crate) root_leaf: ObjectName,
    pub(crate) root_fd: Arc<OwnedFd>,
    pub(crate) identity: RootIdentity,
    pub(crate) faults: FaultController,
    pub(crate) requested_root: PathBuf,
}

#[derive(Clone)]
pub(crate) struct DirectoryHandle {
    pub(crate) anchor: Arc<Anchor>,
    pub(crate) fd: Arc<OwnedFd>,
    pub(crate) identity: ObjectIdentity,
    pub(crate) bindings: Arc<Vec<DirectoryBinding>>,
}

#[derive(Clone)]
pub(crate) struct DirectoryBinding {
    parent: Arc<OwnedFd>,
    name: ObjectName,
    identity: ObjectIdentity,
}

impl DirectoryHandle {
    pub(crate) fn root(anchor: Arc<Anchor>) -> Self {
        let identity = ObjectIdentity {
            device: anchor.identity.device,
            inode: anchor.identity.inode,
        };
        Self {
            fd: Arc::clone(&anchor.root_fd),
            anchor,
            identity,
            bindings: Arc::new(Vec::new()),
        }
    }

    pub(crate) fn child(&self, fd: OwnedFd, name: ObjectName, identity: ObjectIdentity) -> Self {
        let mut bindings = self.bindings.as_ref().clone();
        bindings.push(DirectoryBinding {
            parent: Arc::clone(&self.fd),
            name,
            identity,
        });
        Self {
            anchor: Arc::clone(&self.anchor),
            fd: Arc::new(fd),
            identity,
            bindings: Arc::new(bindings),
        }
    }

    pub(crate) fn context(
        &self,
        name: Option<&ObjectName>,
        operation: StorageOperation,
        step: DurabilityStep,
    ) -> StorageContext {
        self.context_with_object(name, None, operation, step)
    }

    pub(crate) fn context_with_object(
        &self,
        name: Option<&ObjectName>,
        object: Option<crate::ContentDigest>,
        operation: StorageOperation,
        step: DurabilityStep,
    ) -> StorageContext {
        let mut context = StorageContext::new(self.anchor.identity, name, object, operation, step);
        context.requested_root = Some(self.anchor.requested_root.clone());
        context
    }

    pub(crate) fn validate(&self, context: &StorageContext) -> StorageResult<()> {
        self.anchor.validate_root(context)?;
        for binding in self.bindings.iter() {
            binding.validate(context)?;
        }
        let stat = fstat(&self.fd).map_err(|source| StorageError::Io {
            context: context.clone(),
            source: source.into(),
        })?;
        let inspected = inspect_private(&stat, context)?;
        if inspected.identity != self.identity {
            return Err(StorageError::IdentityChanged {
                expected: self.identity,
                actual: Some(inspected.identity),
                context: context.clone(),
            });
        }
        Ok(())
    }

    pub(crate) fn same_anchor(&self, other: &Arc<Anchor>) -> bool {
        Arc::ptr_eq(&self.anchor, other)
    }
}

impl Anchor {
    pub(crate) fn context(
        &self,
        name: Option<&ObjectName>,
        operation: StorageOperation,
        step: DurabilityStep,
    ) -> StorageContext {
        let mut context = StorageContext::new(self.identity, name, None, operation, step);
        context.requested_root = Some(self.requested_root.clone());
        context
    }

    pub(crate) fn validate_root(&self, context: &StorageContext) -> StorageResult<()> {
        let mut context = context.clone();
        context.component = Some(self.root_leaf.clone());
        let named = statat(
            &self.root_parent,
            self.root_leaf.as_os_str(),
            AtFlags::SYMLINK_NOFOLLOW,
        );
        let stat = match named {
            Ok(stat) => stat,
            Err(Errno::NOENT) => {
                return Err(StorageError::RootChanged {
                    expected: self.identity,
                    actual: None,
                    context,
                });
            }
            Err(source) => {
                return Err(StorageError::Io {
                    context,
                    source: source.into(),
                });
            }
        };
        let actual_identity = identity(&stat);
        let actual = RootIdentity {
            device: actual_identity.device,
            inode: actual_identity.inode,
        };
        if actual != self.identity {
            return Err(StorageError::RootChanged {
                expected: self.identity,
                actual: Some(actual),
                context,
            });
        }
        let inspected = inspect_private(&stat, &context)?;
        if inspected.mode != PRIVATE_DIRECTORY_MODE {
            return Err(StorageError::Corruption {
                reason: "root mode changed",
                context,
            });
        }
        let held = fstat(&self.root_fd).map_err(|source| StorageError::Io {
            context: context.clone(),
            source: source.into(),
        })?;
        if identity(&held) != actual_identity {
            return Err(StorageError::RootChanged {
                expected: self.identity,
                actual: Some(actual),
                context,
            });
        }
        Ok(())
    }
}

impl DirectoryBinding {
    fn validate(&self, context: &StorageContext) -> StorageResult<()> {
        let mut context = context.clone();
        context.component = Some(self.name.clone());
        let stat = statat(
            &self.parent,
            self.name.as_os_str(),
            AtFlags::SYMLINK_NOFOLLOW,
        );
        let stat = match stat {
            Ok(stat) => stat,
            Err(Errno::NOENT) => {
                return Err(StorageError::IdentityChanged {
                    expected: self.identity,
                    actual: None,
                    context,
                });
            }
            Err(source) => {
                return Err(StorageError::Io {
                    context,
                    source: source.into(),
                });
            }
        };
        let actual = identity(&stat);
        if actual != self.identity {
            return Err(StorageError::IdentityChanged {
                expected: self.identity,
                actual: Some(actual),
                context,
            });
        }
        let inspected = inspect_private(&stat, &context)?;
        if inspected.identity != self.identity {
            return Err(StorageError::IdentityChanged {
                expected: self.identity,
                actual: Some(inspected.identity),
                context,
            });
        }
        Ok(())
    }
}

pub(crate) fn open_directory(parent: &OwnedFd, name: &ObjectName) -> rustix::io::Result<OwnedFd> {
    openat(
        parent,
        name.as_os_str(),
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        rustix::fs::Mode::empty(),
    )
}
