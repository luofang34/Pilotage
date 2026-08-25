use std::fs::File;
use std::io::Write;
use std::os::unix::ffi::OsStrExt;

use rustix::fd::OwnedFd;
use rustix::fs::{OFlags, fchmod, fstat, openat};
use rustix::io::Errno;

use crate::{
    DurabilityStep, ExactObject, ObjectName, OwnedTemporary, StorageContext, StorageError,
    StorageOperation, StorageResult,
};

use super::barrier::{sync_file, syscall};
use super::directory::DurableDirectory;
use super::metadata::{file_mode, inspect_temporary};
use super::objects;
use super::writer::WriterLease;

mod publication;
mod removal;

pub(crate) use publication::{publish_link, recover_linked_publication};

const TEMPORARY_PREFIX: &[u8] = b".pilotage-tmp-";

pub(crate) fn create(
    directory: &DurableDirectory,
    lease: &WriterLease,
    operation: StorageOperation,
    object: &ExactObject,
) -> StorageResult<OwnedTemporary> {
    let (name, fd) = create_unique(directory, lease, operation, object)?;
    let mut file = File::from(fd);
    let write = context(
        directory,
        &name,
        object,
        operation,
        DurabilityStep::Creation,
    );
    file.write_all(object.bytes())
        .map_err(|source| StorageError::Io {
            context: write,
            source,
        })?;
    let fd = OwnedFd::from(file);
    fchmod(&fd, file_mode()).map_err(|source| StorageError::Io {
        context: context(
            directory,
            &name,
            object,
            operation,
            DurabilityStep::Creation,
        ),
        source: source.into(),
    })?;
    let data = context(
        directory,
        &name,
        object,
        operation,
        DurabilityStep::ObjectData,
    );
    sync_file(&fd, &directory.handle.anchor.faults, &data)?;
    let stat = fstat(&fd).map_err(|source| StorageError::Io {
        context: data.clone(),
        source: source.into(),
    })?;
    let inspected = inspect_temporary(&stat, &data)?;
    if inspected.link_count != 1 {
        return Err(StorageError::LinkedObject {
            actual: inspected.link_count,
            context: data,
        });
    }
    let temporary = OwnedTemporary {
        name,
        identity: inspected.identity,
        object: object.clone(),
        owner_root: directory.handle.anchor.identity,
        owner_directory: directory.handle.identity,
    };
    verify_owned(directory, &temporary, operation)?;
    let after = context(
        directory,
        &temporary.name,
        &temporary.object,
        operation,
        DurabilityStep::AfterMutation,
    );
    lease.validate_for(&directory.handle, &after)?;
    Ok(temporary)
}

pub(crate) fn inspect_owned(
    directory: &DurableDirectory,
    name: &ObjectName,
    maximum_bytes: usize,
) -> StorageResult<OwnedTemporary> {
    require_temporary_name(name, directory)?;
    let object = objects::read_exact(&directory.handle, name, maximum_bytes)?;
    let inspected = objects::inspect(&directory.handle, name)?;
    Ok(OwnedTemporary {
        name: name.clone(),
        identity: inspected.identity,
        object,
        owner_root: directory.handle.anchor.identity,
        owner_directory: directory.handle.identity,
    })
}

pub(crate) fn cleanup(
    directory: &DurableDirectory,
    lease: &WriterLease,
    temporary: &OwnedTemporary,
) -> StorageResult<()> {
    let operation = StorageOperation::RemoveTemporary;
    let context = context(
        directory,
        &temporary.name,
        &temporary.object,
        operation,
        DurabilityStep::BeforeMutation,
    );
    lease.validate_for(&directory.handle, &context)?;
    verify_owned(directory, temporary, operation)?;
    let inspected = objects::inspect(&directory.handle, &temporary.name)?;
    if inspected.link_count != 1 {
        return Err(StorageError::LinkedObject {
            actual: inspected.link_count,
            context,
        });
    }
    removal::unlink_exact(
        directory,
        lease,
        &temporary.name,
        temporary.identity,
        &temporary.object,
        1,
        operation,
    )
}

pub(crate) fn cleanup_unlinked(
    directory: &DurableDirectory,
    lease: &WriterLease,
    maximum_objects: usize,
    maximum_bytes: usize,
) -> StorageResult<usize> {
    let before = directory.handle.context(
        None,
        StorageOperation::RemoveTemporary,
        DurabilityStep::BeforeMutation,
    );
    lease.validate_for(&directory.handle, &before)?;
    let temporaries = directory
        .list()?
        .into_iter()
        .filter(is_temporary_name)
        .collect::<Vec<_>>();
    if temporaries.len() > maximum_objects {
        return Err(StorageError::Corruption {
            reason: "the temporary object count exceeds the recovery limit",
            context: directory.handle.context(
                None,
                StorageOperation::RemoveTemporary,
                DurabilityStep::Selection,
            ),
        });
    }
    for name in &temporaries {
        let temporary = inspect_owned(directory, name, maximum_bytes)?;
        cleanup(directory, lease, &temporary)?;
    }
    let after = directory.handle.context(
        None,
        StorageOperation::RemoveTemporary,
        DurabilityStep::AfterMutation,
    );
    lease.validate_for(&directory.handle, &after)?;
    Ok(temporaries.len())
}

fn create_unique(
    directory: &DurableDirectory,
    lease: &WriterLease,
    operation: StorageOperation,
    object: &ExactObject,
) -> StorageResult<(ObjectName, OwnedFd)> {
    for _ in 0..64 {
        let name = lease.next_temporary_name()?;
        let create = context(
            directory,
            &name,
            object,
            operation,
            DurabilityStep::Creation,
        );
        lease.validate_for(&directory.handle, &create)?;
        match syscall(&directory.handle.anchor.faults, &create, || {
            openat(
                &directory.handle.fd,
                name.as_os_str(),
                OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                file_mode(),
            )
        }) {
            Ok(fd) => return Ok((name, fd)),
            Err(StorageError::Io { source, .. })
                if source.raw_os_error() == Some(Errno::EXIST.raw_os_error()) => {}
            Err(error) => return Err(error),
        }
    }
    Err(StorageError::Corruption {
        reason: "temporary name space is exhausted",
        context: directory.handle.context_with_object(
            None,
            Some(object.digest()),
            operation,
            DurabilityStep::Creation,
        ),
    })
}

pub(crate) fn verify_owned(
    directory: &DurableDirectory,
    temporary: &OwnedTemporary,
    operation: StorageOperation,
) -> StorageResult<()> {
    let ownership = context(
        directory,
        &temporary.name,
        &temporary.object,
        operation,
        DurabilityStep::ObjectReadback,
    );
    if temporary.owner_root != directory.handle.anchor.identity {
        return Err(StorageError::Corruption {
            reason: "temporary token belongs to a different storage root",
            context: ownership,
        });
    }
    if temporary.owner_directory != directory.handle.identity {
        return Err(StorageError::IdentityChanged {
            expected: temporary.owner_directory,
            actual: Some(directory.handle.identity),
            context: ownership,
        });
    }
    let actual = objects::read_exact(
        &directory.handle,
        &temporary.name,
        temporary.object.bytes().len(),
    )
    .map_err(|error| {
        error.at_object(
            operation,
            temporary.object.digest(),
            DurabilityStep::ObjectReadback,
        )
    })?;
    if actual != temporary.object {
        return Err(StorageError::ContentMismatch {
            context: context(
                directory,
                &temporary.name,
                &temporary.object,
                operation,
                DurabilityStep::ObjectReadback,
            ),
        });
    }
    let inspected = objects::inspect(&directory.handle, &temporary.name).map_err(|error| {
        error.at_object(
            operation,
            temporary.object.digest(),
            DurabilityStep::ObjectReadback,
        )
    })?;
    if inspected.identity != temporary.identity {
        return Err(StorageError::IdentityChanged {
            expected: temporary.identity,
            actual: Some(inspected.identity),
            context: context(
                directory,
                &temporary.name,
                &temporary.object,
                operation,
                DurabilityStep::ObjectReadback,
            ),
        });
    }
    Ok(())
}

fn require_temporary_name(name: &ObjectName, directory: &DurableDirectory) -> StorageResult<()> {
    if !is_temporary_name(name) {
        return Err(StorageError::Corruption {
            reason: "name is not an owned temporary component",
            context: directory.handle.context(
                Some(name),
                StorageOperation::InspectObject,
                DurabilityStep::Selection,
            ),
        });
    }
    Ok(())
}

pub(crate) fn reject_reserved_destination(
    directory: &DurableDirectory,
    name: &ObjectName,
    object: &ExactObject,
    operation: StorageOperation,
) -> StorageResult<()> {
    if has_temporary_prefix(name) || WriterLease::is_writer_lock_name(name) {
        return Err(StorageError::InvalidObjectName {
            name: name.as_os_str().into(),
            context: directory.handle.context_with_object(
                Some(name),
                Some(object.digest()),
                operation,
                DurabilityStep::Selection,
            ),
        });
    }
    Ok(())
}

pub(super) fn is_temporary_name(name: &ObjectName) -> bool {
    let bytes = name.as_os_str().as_bytes();
    let Some(remainder) = bytes.strip_prefix(TEMPORARY_PREFIX) else {
        return false;
    };
    let Some(separator) = remainder.iter().position(|byte| *byte == b'-') else {
        return false;
    };
    let (process, counter) = remainder.split_at(separator);
    let counter = &counter[1..];
    !process.is_empty()
        && process.iter().all(u8::is_ascii_digit)
        && counter.len() == 16
        && counter
            .iter()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(byte))
}

fn has_temporary_prefix(name: &ObjectName) -> bool {
    name.as_os_str().as_bytes().starts_with(TEMPORARY_PREFIX)
}

pub(super) fn context(
    directory: &DurableDirectory,
    name: &ObjectName,
    object: &ExactObject,
    operation: StorageOperation,
    step: DurabilityStep,
) -> StorageContext {
    directory
        .handle
        .context_with_object(Some(name), Some(object.digest()), operation, step)
}
