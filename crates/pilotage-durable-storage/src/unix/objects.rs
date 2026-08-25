use std::fs::File;
use std::io::{Read, Seek, SeekFrom};

use rustix::fd::OwnedFd;
use rustix::fs::{AtFlags, Mode, OFlags, fstat, openat, statat};

use crate::{
    ContentDigest, DurabilityStep, ExactObject, ObjectInspection, ObjectName, PutOutcome,
    StorageError, StorageOperation, StorageResult,
};

use super::anchor::DirectoryHandle;
use super::barrier::sync_directory;
use super::directory::DurableDirectory;
use super::metadata::{inspect_private, inspect_temporary};
use super::temporary;
use super::writer::WriterLease;

pub(crate) fn inspect(
    directory: &DirectoryHandle,
    name: &ObjectName,
) -> StorageResult<ObjectInspection> {
    let context = directory.context(
        Some(name),
        StorageOperation::InspectObject,
        DurabilityStep::Selection,
    );
    directory.validate(&context)?;
    let stat =
        statat(&directory.fd, name.as_os_str(), AtFlags::SYMLINK_NOFOLLOW).map_err(|source| {
            StorageError::Io {
                context: context.clone(),
                source: source.into(),
            }
        })?;
    let inspected = inspect_private(&stat, &context)?;
    directory.validate(&context)?;
    Ok(inspected)
}

pub(crate) fn read_exact(
    directory: &DirectoryHandle,
    name: &ObjectName,
    maximum_bytes: usize,
) -> StorageResult<ExactObject> {
    read_exact_with_links(directory, name, maximum_bytes, 1)
}

pub(crate) fn read_digest(
    directory: &DirectoryHandle,
    name: &ObjectName,
    expected: ContentDigest,
    maximum_bytes: usize,
) -> StorageResult<ExactObject> {
    let actual = read_exact(directory, name, maximum_bytes).map_err(|error| {
        error.at_object(
            StorageOperation::ReadObject,
            expected,
            DurabilityStep::ObjectReadback,
        )
    })?;
    if actual.digest() == expected {
        Ok(actual)
    } else {
        Err(StorageError::ContentMismatch {
            context: directory.context_with_object(
                Some(name),
                Some(expected),
                StorageOperation::ReadObject,
                DurabilityStep::ObjectReadback,
            ),
        })
    }
}

pub(crate) fn read_exact_with_links(
    directory: &DirectoryHandle,
    name: &ObjectName,
    maximum_bytes: usize,
    expected_links: u64,
) -> StorageResult<ExactObject> {
    let context = directory.context(
        Some(name),
        StorageOperation::ReadObject,
        DurabilityStep::ObjectReadback,
    );
    directory.validate(&context)?;
    let named_before =
        statat(&directory.fd, name.as_os_str(), AtFlags::SYMLINK_NOFOLLOW).map_err(|source| {
            StorageError::Io {
                context: context.clone(),
                source: source.into(),
            }
        })?;
    let inspected = inspect_temporary(&named_before, &context)?;
    if inspected.link_count != expected_links {
        return Err(StorageError::LinkedObject {
            actual: inspected.link_count,
            context,
        });
    }
    enforce_limit(inspected.size, maximum_bytes, &context)?;
    let fd = open_regular(directory, name, &context)?;
    let opened = verify_open_temporary_identity(&fd, inspected.identity, &context)?;
    if opened.link_count != expected_links {
        return Err(StorageError::LinkedObject {
            actual: opened.link_count,
            context,
        });
    }
    let mut file = File::from(fd);
    let first = read_limited(&mut file, maximum_bytes, &context)?;
    file.seek(SeekFrom::Start(0))
        .map_err(|source| StorageError::Io {
            context: context.clone(),
            source,
        })?;
    let second = read_limited(&mut file, maximum_bytes, &context)?;
    if first != second || first.len() as u64 != inspected.size {
        return Err(StorageError::ContentMismatch {
            context: context.clone(),
        });
    }
    let held_after = verify_open_temporary_identity(&file, inspected.identity, &context)?;
    let named_after =
        statat(&directory.fd, name.as_os_str(), AtFlags::SYMLINK_NOFOLLOW).map_err(|source| {
            StorageError::Io {
                context: context.clone(),
                source: source.into(),
            }
        })?;
    let after = inspect_temporary(&named_after, &context)?;
    if after.identity != inspected.identity {
        return Err(StorageError::IdentityChanged {
            expected: inspected.identity,
            actual: Some(after.identity),
            context,
        });
    }
    if held_after.link_count != expected_links
        || after.link_count != expected_links
        || held_after.size != inspected.size
        || after.size != inspected.size
        || first.len() as u64 != after.size
    {
        return Err(StorageError::ContentMismatch {
            context: context.clone(),
        });
    }
    directory.validate(&context)?;
    Ok(ExactObject::from_bytes(first))
}

pub(crate) fn put_immutable(
    directory: &DurableDirectory,
    lease: &WriterLease,
    name: &ObjectName,
    object: &ExactObject,
) -> StorageResult<PutOutcome> {
    temporary::reject_reserved_destination(
        directory,
        name,
        object,
        StorageOperation::PublishImmutable,
    )?;
    let before = directory.handle.context_with_object(
        Some(name),
        Some(object.digest()),
        StorageOperation::PublishImmutable,
        DurabilityStep::BeforeMutation,
    );
    lease.validate_for(&directory.handle, &before)?;
    if temporary::recover_linked_publication(directory, lease, name, object)? {
        make_existing_durable(&directory.handle, name, object)?;
        let after = directory.handle.context_with_object(
            Some(name),
            Some(object.digest()),
            StorageOperation::PublishImmutable,
            DurabilityStep::AfterMutation,
        );
        lease.validate_for(&directory.handle, &after)?;
        return Ok(PutOutcome::AlreadyExact);
    }
    if exists_raw(&directory.handle, name, &before)? {
        verify_exact(
            &directory.handle,
            name,
            object,
            StorageOperation::PublishImmutable,
        )?;
        make_existing_durable(&directory.handle, name, object)?;
        let after = directory.handle.context_with_object(
            Some(name),
            Some(object.digest()),
            StorageOperation::PublishImmutable,
            DurabilityStep::AfterMutation,
        );
        lease.validate_for(&directory.handle, &after)?;
        return Ok(PutOutcome::AlreadyExact);
    }
    let temporary =
        temporary::create(directory, lease, StorageOperation::PublishImmutable, object)?;
    temporary::publish_link(directory, lease, &temporary, name, object)?;
    make_existing_durable(&directory.handle, name, object)?;
    let after = directory.handle.context_with_object(
        Some(name),
        Some(object.digest()),
        StorageOperation::PublishImmutable,
        DurabilityStep::AfterMutation,
    );
    lease.validate_for(&directory.handle, &after)?;
    Ok(PutOutcome::Published)
}

pub(crate) fn verify_exact(
    directory: &DirectoryHandle,
    name: &ObjectName,
    expected: &ExactObject,
    operation: StorageOperation,
) -> StorageResult<()> {
    let actual = read_exact(directory, name, expected.bytes().len()).map_err(|error| {
        error.at_object(operation, expected.digest(), DurabilityStep::ObjectReadback)
    })?;
    if actual.digest() != expected.digest() || actual.bytes() != expected.bytes() {
        let context = directory.context_with_object(
            Some(name),
            Some(expected.digest()),
            operation,
            DurabilityStep::ObjectReadback,
        );
        return Err(StorageError::ContentMismatch { context });
    }
    Ok(())
}

pub(crate) fn exists_raw(
    directory: &DirectoryHandle,
    name: &ObjectName,
    context: &crate::StorageContext,
) -> StorageResult<bool> {
    match statat(&directory.fd, name.as_os_str(), AtFlags::SYMLINK_NOFOLLOW) {
        Ok(stat) => {
            inspect_private(&stat, context)?;
            Ok(true)
        }
        Err(rustix::io::Errno::NOENT) => Ok(false),
        Err(source) => Err(StorageError::Io {
            context: context.clone(),
            source: source.into(),
        }),
    }
}

pub(crate) fn make_existing_durable(
    directory: &DirectoryHandle,
    name: &ObjectName,
    object: &ExactObject,
) -> StorageResult<()> {
    verify_exact(directory, name, object, StorageOperation::PublishImmutable)?;
    let context = directory.context_with_object(
        Some(name),
        Some(object.digest()),
        StorageOperation::PublishImmutable,
        DurabilityStep::ObjectData,
    );
    let fd = open_regular(directory, name, &context)?;
    let named =
        statat(&directory.fd, name.as_os_str(), AtFlags::SYMLINK_NOFOLLOW).map_err(|source| {
            StorageError::Io {
                context: context.clone(),
                source: source.into(),
            }
        })?;
    let named_inspection = inspect_private(&named, &context)?;
    verify_open_identity(&fd, named_inspection.identity, &context)?;
    super::barrier::sync_file(&fd, &directory.anchor.faults, &context)?;
    let parent = crate::StorageContext {
        step: DurabilityStep::ParentDirectory,
        ..context
    };
    sync_directory(&directory.fd, &directory.anchor.faults, &parent)?;
    verify_exact(directory, name, object, StorageOperation::PublishImmutable)
}

pub(crate) fn open_regular(
    directory: &DirectoryHandle,
    name: &ObjectName,
    context: &crate::StorageContext,
) -> StorageResult<OwnedFd> {
    openat(
        &directory.fd,
        name.as_os_str(),
        OFlags::RDONLY | OFlags::NONBLOCK | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|source| StorageError::Io {
        context: context.clone(),
        source: source.into(),
    })
}

pub(crate) fn verify_open_identity(
    fd: impl std::os::fd::AsFd,
    expected: crate::ObjectIdentity,
    context: &crate::StorageContext,
) -> StorageResult<ObjectInspection> {
    let stat = fstat(fd).map_err(|source| StorageError::Io {
        context: context.clone(),
        source: source.into(),
    })?;
    let inspected = inspect_private(&stat, context)?;
    if inspected.identity != expected {
        return Err(StorageError::IdentityChanged {
            expected,
            actual: Some(inspected.identity),
            context: context.clone(),
        });
    }
    Ok(inspected)
}

fn verify_open_temporary_identity(
    fd: impl std::os::fd::AsFd,
    expected: crate::ObjectIdentity,
    context: &crate::StorageContext,
) -> StorageResult<ObjectInspection> {
    let stat = fstat(fd).map_err(|source| StorageError::Io {
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
    Ok(inspected)
}

fn enforce_limit(
    actual: u64,
    maximum_bytes: usize,
    context: &crate::StorageContext,
) -> StorageResult<()> {
    if actual > maximum_bytes as u64 {
        return Err(StorageError::ObjectTooLarge {
            limit: maximum_bytes,
            actual,
            context: context.clone(),
        });
    }
    Ok(())
}

fn read_limited(
    file: &mut File,
    maximum_bytes: usize,
    context: &crate::StorageContext,
) -> StorageResult<Vec<u8>> {
    let take = maximum_bytes
        .checked_add(1)
        .ok_or_else(|| StorageError::Corruption {
            reason: "read limit cannot be represented",
            context: context.clone(),
        })?;
    let mut bytes = Vec::new();
    file.take(take as u64)
        .read_to_end(&mut bytes)
        .map_err(|source| StorageError::Io {
            context: context.clone(),
            source,
        })?;
    if bytes.len() > maximum_bytes {
        return Err(StorageError::ObjectTooLarge {
            limit: maximum_bytes,
            actual: bytes.len() as u64,
            context: context.clone(),
        });
    }
    Ok(bytes)
}
