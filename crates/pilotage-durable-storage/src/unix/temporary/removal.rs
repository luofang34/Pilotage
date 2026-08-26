use rustix::fs::{AtFlags, statat, unlinkat};
use rustix::io::Errno;

use crate::{
    DurabilityStep, ExactObject, ObjectIdentity, ObjectName, StorageContext, StorageError,
    StorageOperation, StorageResult,
};

use super::super::barrier::{sync_directory, syscall};
use super::super::directory::DurableDirectory;
use super::super::metadata::inspect_temporary;
use super::super::writer::WriterLease;
use super::context;

#[derive(Clone, Copy)]
struct UnlinkTarget<'a> {
    name: &'a ObjectName,
    identity: ObjectIdentity,
    object: &'a ExactObject,
    link_count: u64,
}

pub(super) fn unlink_exact(
    directory: &DurableDirectory,
    lease: &WriterLease,
    name: &ObjectName,
    expected: ObjectIdentity,
    object: &ExactObject,
    expected_links: u64,
    operation: StorageOperation,
) -> StorageResult<()> {
    let target = UnlinkTarget {
        name,
        identity: expected,
        object,
        link_count: expected_links,
    };
    let deletion = directory.handle.context_with_object(
        Some(name),
        Some(object.digest()),
        operation,
        DurabilityStep::Deletion,
    );
    target.prepare(directory, lease, &deletion)?;
    let unlinked = syscall(&directory.handle.anchor.faults, &deletion, || {
        unlinkat(
            &directory.handle.fd,
            target.name.as_os_str(),
            AtFlags::empty(),
        )
    });
    if let Err(original) = unlinked {
        resolve_unlink(directory, lease, target, &deletion, original)?;
    }
    finish_removal(directory, lease, name, object, operation, deletion)
}

impl UnlinkTarget<'_> {
    fn prepare(
        self,
        directory: &DurableDirectory,
        lease: &WriterLease,
        context: &StorageContext,
    ) -> StorageResult<()> {
        lease.validate_for(&directory.handle, context)?;
        let actual = super::super::objects::read_exact_with_links(
            &directory.handle,
            self.name,
            self.object.bytes().len(),
            self.link_count,
        )
        .map_err(|error| {
            error.at_object(
                context.operation,
                self.object.digest(),
                DurabilityStep::ObjectReadback,
            )
        })?;
        if actual != *self.object {
            return Err(StorageError::ContentMismatch {
                context: context.clone(),
            });
        }
        validate_name(
            directory,
            self.name,
            self.identity,
            self.link_count,
            context,
        )
    }
}

fn validate_name(
    directory: &DurableDirectory,
    name: &ObjectName,
    expected: ObjectIdentity,
    expected_links: u64,
    context: &StorageContext,
) -> StorageResult<()> {
    let stat = statat(
        &directory.handle.fd,
        name.as_os_str(),
        AtFlags::SYMLINK_NOFOLLOW,
    )
    .map_err(|source| StorageError::Io {
        context: context.clone(),
        source: source.into(),
    })?;
    let inspected = inspect_temporary(&stat, context)?;
    if inspected.identity != expected {
        return Err(StorageError::IdentityChanged {
            expected,
            actual: Some(inspected.identity),
            context: context.clone(),
        });
    }
    if inspected.link_count != expected_links {
        return Err(StorageError::LinkedObject {
            actual: inspected.link_count,
            context: context.clone(),
        });
    }
    Ok(())
}

fn resolve_unlink(
    directory: &DurableDirectory,
    lease: &WriterLease,
    target: UnlinkTarget<'_>,
    context: &StorageContext,
    original: StorageError,
) -> StorageResult<()> {
    match statat(
        &directory.handle.fd,
        target.name.as_os_str(),
        AtFlags::SYMLINK_NOFOLLOW,
    ) {
        Err(Errno::NOENT) => Ok(()),
        Ok(_) => {
            target.prepare(directory, lease, context)?;
            let retried = syscall(&directory.handle.anchor.faults, context, || {
                unlinkat(
                    &directory.handle.fd,
                    target.name.as_os_str(),
                    AtFlags::empty(),
                )
            });
            resolve_retry(directory, target.name, context, original, retried)
        }
        Err(source) => Err(StorageError::Io {
            context: context.clone(),
            source: source.into(),
        }),
    }
}

fn resolve_retry(
    directory: &DurableDirectory,
    name: &ObjectName,
    context: &StorageContext,
    original: StorageError,
    retried: StorageResult<()>,
) -> StorageResult<()> {
    let Err(retry) = retried else {
        return Ok(());
    };
    match statat(
        &directory.handle.fd,
        name.as_os_str(),
        AtFlags::SYMLINK_NOFOLLOW,
    ) {
        Err(Errno::NOENT) => Ok(()),
        Ok(_) => Err(retry),
        Err(source) => Err(StorageError::AmbiguousCommit {
            context: context.clone(),
            prior: Some(Box::new(original)),
            source: Box::new(StorageError::Io {
                context: context.clone(),
                source: source.into(),
            }),
        }),
    }
}

fn finish_removal(
    directory: &DurableDirectory,
    lease: &WriterLease,
    name: &ObjectName,
    object: &ExactObject,
    operation: StorageOperation,
    deletion: StorageContext,
) -> StorageResult<()> {
    let parent = StorageContext {
        step: DurabilityStep::ParentDirectory,
        ..deletion
    };
    let first = sync_directory(
        &directory.handle.fd,
        &directory.handle.anchor.faults,
        &parent,
    );
    if let Err(first_error) = first {
        return recover_barrier(
            directory,
            lease,
            name,
            object,
            operation,
            parent,
            first_error,
        );
    }
    validate_after(directory, lease, name, object, operation)
}

fn recover_barrier(
    directory: &DurableDirectory,
    lease: &WriterLease,
    name: &ObjectName,
    object: &ExactObject,
    operation: StorageOperation,
    parent: StorageContext,
    first: StorageError,
) -> StorageResult<()> {
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
    validate_after(directory, lease, name, object, operation)
}

fn validate_after(
    directory: &DurableDirectory,
    lease: &WriterLease,
    name: &ObjectName,
    object: &ExactObject,
    operation: StorageOperation,
) -> StorageResult<()> {
    let after = context(
        directory,
        name,
        object,
        operation,
        DurabilityStep::AfterMutation,
    );
    super::super::remove::validate_absent_after(lease, directory, name, after)
}
