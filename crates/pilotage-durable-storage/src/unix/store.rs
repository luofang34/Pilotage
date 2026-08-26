use std::path::{Component, Path};
use std::sync::Arc;

use rustix::fd::OwnedFd;
use rustix::fs::{ABS, AtFlags, Mode, OFlags, fchmod, fstat, mkdirat, openat, statat};
use rustix::io::Errno;

use crate::fault::FaultController;
use crate::{
    DurabilityStep, ObjectName, RootIdentity, StorageContext, StorageError, StorageOperation,
    StorageResult,
};

use super::anchor::{Anchor, DirectoryHandle, open_directory};
use super::barrier::{sync_directory, syscall};
use super::directory::DurableDirectory;
use super::metadata::{PRIVATE_DIRECTORY_MODE, directory_mode, identity, inspect_private};
use super::writer::WriterLease;

/// An anchored private durable-storage session.
#[derive(Clone)]
pub struct DurableStore {
    pub(crate) anchor: Arc<Anchor>,
}

impl DurableStore {
    /// Open or create one exact private Unix storage root.
    pub fn open_or_create(path: &Path) -> StorageResult<Self> {
        Self::open_with_controller(path, FaultController::default())
    }

    /// Open or create a store with deterministic storage faults.
    #[cfg(any(test, feature = "fault-injection"))]
    pub fn open_or_create_with_faults(path: &Path, faults: FaultController) -> StorageResult<Self> {
        Self::open_with_controller(path, faults)
    }

    /// Get the device and inode bound to this session.
    #[must_use]
    pub fn root_identity(&self) -> RootIdentity {
        self.anchor.identity
    }

    /// Get the anchored root directory.
    #[must_use]
    pub fn root_directory(&self) -> DurableDirectory {
        DurableDirectory::new(DirectoryHandle::root(Arc::clone(&self.anchor)))
    }

    /// Acquire the one nonblocking writer lease for this root.
    pub fn acquire_writer(&self) -> StorageResult<WriterLease> {
        WriterLease::acquire(Arc::clone(&self.anchor))
    }

    fn open_with_controller(path: &Path, faults: FaultController) -> StorageResult<Self> {
        let context = StorageContext::root_open_at(path);
        let (parent, leaf) = open_parent(path, &context)?;
        let root_fd = open_or_create_root(&parent, &leaf, &faults, &context)?;
        let mut selected = context.clone();
        selected.component = Some(leaf.clone());
        let stat = fstat(&root_fd).map_err(|source| StorageError::Io {
            context: selected.clone(),
            source: source.into(),
        })?;
        let object_identity = identity(&stat);
        let root_identity = RootIdentity {
            device: object_identity.device,
            inode: object_identity.inode,
        };
        let mut bound = selected;
        bound.root = Some(root_identity);
        bound.component = Some(leaf.clone());
        let inspected = inspect_private(&stat, &bound)?;
        if inspected.mode != PRIVATE_DIRECTORY_MODE {
            return Err(StorageError::WrongMode {
                required: PRIVATE_DIRECTORY_MODE,
                actual: inspected.mode,
                context: bound,
            });
        }
        let root_barrier = StorageContext {
            step: DurabilityStep::ObjectData,
            ..bound.clone()
        };
        sync_directory(&root_fd, &faults, &root_barrier)?;
        let parent_barrier = StorageContext {
            step: DurabilityStep::ParentDirectory,
            ..bound
        };
        sync_directory(&parent, &faults, &parent_barrier)?;
        let anchor = Arc::new(Anchor {
            root_parent: parent,
            root_leaf: leaf,
            root_fd: Arc::new(root_fd),
            identity: root_identity,
            faults,
            requested_root: path.to_path_buf(),
        });
        let validate = anchor.context(
            None,
            StorageOperation::ValidateRoot,
            DurabilityStep::AfterMutation,
        );
        anchor.validate_root(&validate)?;
        Ok(Self { anchor })
    }
}

fn open_parent(path: &Path, context: &StorageContext) -> StorageResult<(OwnedFd, ObjectName)> {
    if !path.is_absolute() {
        return Err(StorageError::Corruption {
            reason: "storage root must be absolute",
            context: context.clone(),
        });
    }
    let mut names = Vec::new();
    for component in path.components() {
        match component {
            Component::RootDir => {}
            Component::Normal(name) => names.push(
                ObjectName::new(name).map_err(|_| StorageError::invalid_name_at(name, context))?,
            ),
            Component::CurDir | Component::ParentDir | Component::Prefix(_) => {
                return Err(StorageError::Corruption {
                    reason: "storage root contains a traversal component",
                    context: context.clone(),
                });
            }
        }
    }
    let leaf = names.pop().ok_or_else(|| StorageError::Corruption {
        reason: "storage root must have a leaf component",
        context: context.clone(),
    })?;
    let mut parent = openat(
        ABS,
        c"/",
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|source| StorageError::Io {
        context: context.clone(),
        source: source.into(),
    })?;
    for name in names {
        let mut component_context = context.clone();
        component_context.component = Some(name.clone());
        parent = open_directory(&parent, &name).map_err(|source| StorageError::Io {
            context: component_context,
            source: source.into(),
        })?;
    }
    Ok((parent, leaf))
}

fn open_or_create_root(
    parent: &OwnedFd,
    leaf: &ObjectName,
    faults: &FaultController,
    context: &StorageContext,
) -> StorageResult<OwnedFd> {
    let mut context = context.clone();
    context.component = Some(leaf.clone());
    match statat(parent, leaf.as_os_str(), AtFlags::SYMLINK_NOFOLLOW) {
        Ok(_) => return open_existing_root(parent, leaf, &context),
        Err(Errno::NOENT) => {}
        Err(source) => {
            return Err(StorageError::Io {
                context: context.clone(),
                source: source.into(),
            });
        }
    }
    let create_context = StorageContext {
        step: DurabilityStep::Creation,
        ..context.clone()
    };
    let creation = syscall(faults, &create_context, || {
        mkdirat(parent, leaf.as_os_str(), directory_mode())
    });
    let root = match creation {
        Ok(()) => open_directory(parent, leaf).map_err(|source| StorageError::Io {
            context: create_context.clone(),
            source: source.into(),
        })?,
        Err(original) => match statat(parent, leaf.as_os_str(), AtFlags::SYMLINK_NOFOLLOW) {
            Ok(_) => open_existing_root(parent, leaf, &context)?,
            Err(Errno::NOENT) => return Err(original),
            Err(source) => {
                return Err(StorageError::Io {
                    context: create_context,
                    source: source.into(),
                });
            }
        },
    };
    fchmod(&root, directory_mode()).map_err(|source| StorageError::Io {
        context: create_context.clone(),
        source: source.into(),
    })?;
    let root_identity = verify_exact_root(&root, &context)?;
    let mut bound = context;
    bound.root = Some(root_identity);
    let object_context = StorageContext {
        step: DurabilityStep::ObjectData,
        ..bound.clone()
    };
    sync_directory(&root, faults, &object_context)?;
    let parent_context = StorageContext {
        step: DurabilityStep::ParentDirectory,
        ..bound
    };
    sync_directory(parent, faults, &parent_context)?;
    Ok(root)
}

fn open_existing_root(
    parent: &OwnedFd,
    leaf: &ObjectName,
    context: &StorageContext,
) -> StorageResult<OwnedFd> {
    let root = open_directory(parent, leaf).map_err(|source| StorageError::Io {
        context: context.clone(),
        source: source.into(),
    })?;
    verify_exact_root(&root, context)?;
    Ok(root)
}

fn verify_exact_root(root: &OwnedFd, context: &StorageContext) -> StorageResult<RootIdentity> {
    let stat = fstat(root).map_err(|source| StorageError::Io {
        context: context.clone(),
        source: source.into(),
    })?;
    let actual = identity(&stat);
    let mut bound = context.clone();
    let root_identity = RootIdentity {
        device: actual.device,
        inode: actual.inode,
    };
    bound.root = Some(root_identity);
    let inspected = inspect_private(&stat, &bound)?;
    if inspected.mode != PRIVATE_DIRECTORY_MODE {
        return Err(StorageError::WrongMode {
            required: PRIVATE_DIRECTORY_MODE,
            actual: inspected.mode,
            context: bound,
        });
    }
    Ok(root_identity)
}
