use rustix::fs::{AtFlags, statat, unlinkat};
use rustix::io::Errno;

use crate::{
    DurabilityStep, ObjectKind, ObjectName, PrivateTreeManifest, StorageContext, StorageError,
    StorageOperation, StorageResult, types::PrivateTreeNode,
};

use super::barrier::{sync_directory, syscall};
use super::directory::DurableDirectory;
use super::writer::WriterLease;

mod manifest;

pub(crate) use manifest::inspect_private_tree;

impl WriterLease {
    /// Remove one exact private tree manifest.
    pub fn remove_private_tree(
        &self,
        directory: &DurableDirectory,
        manifest: &PrivateTreeManifest,
    ) -> StorageResult<()> {
        let context = directory.handle.context(
            Some(&manifest.root.name),
            StorageOperation::RemoveTree,
            DurabilityStep::BeforeMutation,
        );
        self.validate_for(&directory.handle, &context)?;
        manifest::validate_owner(directory, manifest, &context)?;
        if self.protects_writer_lock(directory, &manifest.root.name) {
            return Err(StorageError::Corruption {
                reason: "writer lease file cannot be removed",
                context,
            });
        }
        manifest::verify(directory, &manifest.root, &context)?;
        remove_node(self, directory, &manifest.root)
    }

    /// Remove one verified owned temporary file.
    pub fn cleanup_owned_temporary(
        &self,
        directory: &DurableDirectory,
        temporary: &crate::OwnedTemporary,
    ) -> StorageResult<()> {
        super::temporary::cleanup(directory, self, temporary)
    }

    fn protects_writer_lock(&self, directory: &DurableDirectory, name: &ObjectName) -> bool {
        std::sync::Arc::ptr_eq(&directory.handle.fd, &self.anchor.root_fd)
            && Self::is_writer_lock_name(name)
    }
}

fn remove_node(
    lease: &WriterLease,
    directory: &DurableDirectory,
    node: &PrivateTreeNode,
) -> StorageResult<()> {
    let context = directory.handle.context(
        Some(&node.name),
        StorageOperation::RemoveTree,
        DurabilityStep::BeforeMutation,
    );
    manifest::verify_exact_node(directory, node, &context)?;
    if node.kind == ObjectKind::Directory {
        let child = manifest::open_manifest_directory(directory, node, &context)?;
        for descendant in &node.children {
            remove_node(lease, &child, descendant)?;
        }
        if !child.list()?.is_empty() {
            return Err(StorageError::Corruption {
                reason: "deletion directory gained an unknown object",
                context,
            });
        }
    }
    lease.validate_for(&directory.handle, &context)?;
    manifest::verify_exact_node(directory, node, &context)?;
    let deletion = node.file.as_ref().map_or_else(
        || {
            directory.handle.context(
                Some(&node.name),
                StorageOperation::RemoveTree,
                DurabilityStep::Deletion,
            )
        },
        |file| {
            directory.handle.context_with_object(
                Some(&node.name),
                Some(file.digest),
                StorageOperation::RemoveTree,
                DurabilityStep::Deletion,
            )
        },
    );
    let flags = if node.kind == ObjectKind::Directory {
        AtFlags::REMOVEDIR
    } else {
        AtFlags::empty()
    };
    let result = syscall(&directory.handle.anchor.faults, &deletion, || {
        unlinkat(&directory.handle.fd, node.name.as_os_str(), flags)
    });
    if let Err(error) = result {
        return recover_unlink(lease, directory, node, deletion, error);
    }
    finish_removed(lease, directory, node, deletion)
}

fn recover_unlink(
    lease: &WriterLease,
    directory: &DurableDirectory,
    node: &PrivateTreeNode,
    deletion: StorageContext,
    original: StorageError,
) -> StorageResult<()> {
    match statat(
        &directory.handle.fd,
        node.name.as_os_str(),
        AtFlags::SYMLINK_NOFOLLOW,
    ) {
        Err(Errno::NOENT) => {
            let barrier = StorageContext {
                step: DurabilityStep::RecoveryBarrier,
                ..deletion
            };
            sync_directory(
                &directory.handle.fd,
                &directory.handle.anchor.faults,
                &barrier,
            )?;
            let mut after = barrier;
            after.step = DurabilityStep::AfterMutation;
            validate_absent_after(lease, directory, &node.name, after)
        }
        Ok(_) => {
            let mut after = deletion.clone();
            after.step = DurabilityStep::AfterMutation;
            lease.validate_for(&directory.handle, &after)?;
            Err(original)
        }
        Err(source) => Err(StorageError::Io {
            context: deletion,
            source: source.into(),
        }),
    }
}

fn finish_removed(
    lease: &WriterLease,
    directory: &DurableDirectory,
    node: &PrivateTreeNode,
    deletion: StorageContext,
) -> StorageResult<()> {
    let parent = StorageContext {
        step: DurabilityStep::ParentDirectory,
        ..deletion
    };
    if let Err(first) = sync_directory(
        &directory.handle.fd,
        &directory.handle.anchor.faults,
        &parent,
    ) {
        match statat(
            &directory.handle.fd,
            node.name.as_os_str(),
            AtFlags::SYMLINK_NOFOLLOW,
        ) {
            Err(Errno::NOENT) => {}
            Ok(_) => {
                return Err(StorageError::Corruption {
                    reason: "removed name reappeared before its durability barrier",
                    context: parent,
                });
            }
            Err(source) => {
                return Err(StorageError::Io {
                    context: parent,
                    source: source.into(),
                });
            }
        }
        let recovery = StorageContext {
            step: DurabilityStep::RecoveryBarrier,
            ..parent
        };
        if let Err(second) = sync_directory(
            &directory.handle.fd,
            &directory.handle.anchor.faults,
            &recovery,
        ) {
            return Err(StorageError::AmbiguousCommit {
                context: recovery,
                prior: Some(Box::new(first)),
                source: Box::new(second),
            });
        }
        let mut after = recovery;
        after.step = DurabilityStep::AfterMutation;
        return validate_absent_after(lease, directory, &node.name, after);
    }
    let mut after = parent;
    after.step = DurabilityStep::AfterMutation;
    validate_absent_after(lease, directory, &node.name, after)
}

pub(crate) fn validate_absent_after(
    lease: &WriterLease,
    directory: &DurableDirectory,
    name: &ObjectName,
    context: StorageContext,
) -> StorageResult<()> {
    lease.validate_for(&directory.handle, &context)?;
    match statat(
        &directory.handle.fd,
        name.as_os_str(),
        AtFlags::SYMLINK_NOFOLLOW,
    ) {
        Err(Errno::NOENT) => Ok(()),
        Ok(_) => Err(StorageError::Corruption {
            reason: "removed name reappeared after its durability barrier",
            context,
        }),
        Err(source) => Err(StorageError::Io {
            context,
            source: source.into(),
        }),
    }
}
