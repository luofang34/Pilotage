use rustix::fs::{AtFlags, Stat, linkat, statat};
use rustix::io::Errno;

use crate::{
    DurabilityStep, ExactObject, ObjectIdentity, ObjectName, OwnedTemporary, StorageContext,
    StorageError, StorageOperation, StorageResult,
};

use super::super::barrier::syscall;
use super::super::directory::DurableDirectory;
use super::super::metadata::{identity, inspect_temporary};
use super::super::objects;
use super::super::writer::WriterLease;
use super::{cleanup, context, is_temporary_name, removal, verify_owned};

pub(crate) fn publish_link(
    directory: &DurableDirectory,
    lease: &WriterLease,
    temporary: &OwnedTemporary,
    destination: &ObjectName,
    expected: &ExactObject,
) -> StorageResult<()> {
    let publish = context(
        directory,
        destination,
        expected,
        StorageOperation::PublishImmutable,
        DurabilityStep::ObjectPublication,
    );
    lease.validate_for(&directory.handle, &publish)?;
    verify_owned(directory, temporary, StorageOperation::PublishImmutable)?;
    let result = syscall(&directory.handle.anchor.faults, &publish, || {
        linkat(
            &directory.handle.fd,
            temporary.name.as_os_str(),
            &directory.handle.fd,
            destination.as_os_str(),
            AtFlags::empty(),
        )
    });
    match result {
        Ok(()) => unlink_linked_temporary(directory, lease, temporary, destination),
        Err(error) => recover_link(directory, lease, temporary, destination, expected, error),
    }
}

pub(crate) fn recover_linked_publication(
    directory: &DurableDirectory,
    lease: &WriterLease,
    destination: &ObjectName,
    expected: &ExactObject,
) -> StorageResult<bool> {
    let before = context(
        directory,
        destination,
        expected,
        StorageOperation::PublishImmutable,
        DurabilityStep::BeforeMutation,
    );
    lease.validate_for(&directory.handle, &before)?;
    let Some(destination_stat) = stat_optional(directory, destination, &before)? else {
        return Ok(false);
    };
    let inspected = inspect_temporary(&destination_stat, &before)?;
    if inspected.link_count == 1 {
        return Ok(false);
    }
    if inspected.link_count != 2 {
        return Err(StorageError::LinkedObject {
            actual: inspected.link_count,
            context: before,
        });
    }
    let temporary_name = find_temporary_link(directory, inspected.identity, expected)?;
    let temporary = OwnedTemporary {
        name: temporary_name,
        identity: inspected.identity,
        object: expected.clone(),
        owner_root: directory.handle.anchor.identity,
        owner_directory: directory.handle.identity,
    };
    unlink_linked_temporary(directory, lease, &temporary, destination)?;
    Ok(true)
}

fn find_temporary_link(
    directory: &DurableDirectory,
    expected_identity: ObjectIdentity,
    expected: &ExactObject,
) -> StorageResult<ObjectName> {
    let mut found = None;
    for name in directory.list()?.into_iter().filter(is_temporary_name) {
        let selection = context(
            directory,
            &name,
            expected,
            StorageOperation::PublishImmutable,
            DurabilityStep::ObjectReadback,
        );
        let Some(stat) = stat_optional(directory, &name, &selection)? else {
            continue;
        };
        if identity(&stat) != expected_identity {
            continue;
        }
        validate_linked_stat(&stat, expected_identity, &selection)?;
        if found.replace(name).is_some() {
            return Err(StorageError::Corruption {
                reason: "linked publication has more than one owned temporary name",
                context: selection,
            });
        }
    }
    found.ok_or_else(|| StorageError::Corruption {
        reason: "linked publication has no exact owned temporary name",
        context: directory.handle.context_with_object(
            None,
            Some(expected.digest()),
            StorageOperation::PublishImmutable,
            DurabilityStep::ObjectReadback,
        ),
    })
}

fn recover_link(
    directory: &DurableDirectory,
    lease: &WriterLease,
    temporary: &OwnedTemporary,
    destination: &ObjectName,
    expected: &ExactObject,
    original: StorageError,
) -> StorageResult<()> {
    let readback = context(
        directory,
        destination,
        expected,
        StorageOperation::PublishImmutable,
        DurabilityStep::ObjectReadback,
    );
    let named = match stat_with_fault(directory, destination, &readback) {
        Ok(value) => value,
        Err(source) => return Err(ambiguous(readback, original, source)),
    };
    match named {
        Some(stat) if identity(&stat) == temporary.identity => {
            unlink_linked_temporary(directory, lease, temporary, destination)
        }
        Some(_) => Err(StorageError::Corruption {
            reason: "immutable destination appeared during publication",
            context: readback,
        }),
        None => {
            cleanup(directory, lease, temporary)?;
            Err(original)
        }
    }
}

fn unlink_linked_temporary(
    directory: &DurableDirectory,
    lease: &WriterLease,
    temporary: &OwnedTemporary,
    destination: &ObjectName,
) -> StorageResult<()> {
    verify_linked_pair(directory, temporary, destination)?;
    removal::unlink_exact(
        directory,
        lease,
        &temporary.name,
        temporary.identity,
        &temporary.object,
        2,
        StorageOperation::PublishImmutable,
    )?;
    objects::verify_exact(
        &directory.handle,
        destination,
        &temporary.object,
        StorageOperation::PublishImmutable,
    )?;
    let after = context(
        directory,
        destination,
        &temporary.object,
        StorageOperation::PublishImmutable,
        DurabilityStep::AfterMutation,
    );
    lease.validate_for(&directory.handle, &after)
}

fn verify_linked_pair(
    directory: &DurableDirectory,
    temporary: &OwnedTemporary,
    destination: &ObjectName,
) -> StorageResult<()> {
    for name in [&temporary.name, destination] {
        let actual = objects::read_exact_with_links(
            &directory.handle,
            name,
            temporary.object.bytes().len(),
            2,
        )
        .map_err(|error| {
            error.at_object(
                StorageOperation::PublishImmutable,
                temporary.object.digest(),
                DurabilityStep::ObjectReadback,
            )
        })?;
        if actual != temporary.object {
            return Err(StorageError::ContentMismatch {
                context: context(
                    directory,
                    name,
                    &temporary.object,
                    StorageOperation::PublishImmutable,
                    DurabilityStep::ObjectReadback,
                ),
            });
        }
        let selection = context(
            directory,
            name,
            &temporary.object,
            StorageOperation::PublishImmutable,
            DurabilityStep::ObjectReadback,
        );
        let stat = stat_required(directory, name, &selection)?;
        validate_linked_stat(&stat, temporary.identity, &selection)?;
    }
    Ok(())
}

fn validate_linked_stat(
    stat: &Stat,
    expected: ObjectIdentity,
    context: &StorageContext,
) -> StorageResult<()> {
    let inspected = inspect_temporary(stat, context)?;
    if inspected.identity != expected {
        return Err(StorageError::IdentityChanged {
            expected,
            actual: Some(inspected.identity),
            context: context.clone(),
        });
    }
    if inspected.link_count != 2 {
        return Err(StorageError::LinkedObject {
            actual: inspected.link_count,
            context: context.clone(),
        });
    }
    Ok(())
}

fn stat_with_fault(
    directory: &DurableDirectory,
    name: &ObjectName,
    context: &StorageContext,
) -> StorageResult<Option<Stat>> {
    syscall(&directory.handle.anchor.faults, context, || {
        match statat(
            &directory.handle.fd,
            name.as_os_str(),
            AtFlags::SYMLINK_NOFOLLOW,
        ) {
            Ok(stat) => Ok(Some(stat)),
            Err(Errno::NOENT) => Ok(None),
            Err(error) => Err(error),
        }
    })
}

fn stat_optional(
    directory: &DurableDirectory,
    name: &ObjectName,
    context: &StorageContext,
) -> StorageResult<Option<Stat>> {
    match statat(
        &directory.handle.fd,
        name.as_os_str(),
        AtFlags::SYMLINK_NOFOLLOW,
    ) {
        Ok(stat) => Ok(Some(stat)),
        Err(Errno::NOENT) => Ok(None),
        Err(source) => Err(StorageError::Io {
            context: context.clone(),
            source: source.into(),
        }),
    }
}

fn stat_required(
    directory: &DurableDirectory,
    name: &ObjectName,
    context: &StorageContext,
) -> StorageResult<Stat> {
    stat_optional(directory, name, context)?.ok_or_else(|| StorageError::Corruption {
        reason: "linked publication name disappeared during validation",
        context: context.clone(),
    })
}

fn ambiguous(context: StorageContext, prior: StorageError, source: StorageError) -> StorageError {
    StorageError::AmbiguousCommit {
        context,
        prior: Some(Box::new(prior)),
        source: Box::new(source),
    }
}
