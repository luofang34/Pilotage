use std::convert::Infallible;

use rustix::fs::{AtFlags, renameat, statat};
#[cfg(any(target_vendor = "apple", target_os = "linux", target_os = "android"))]
use rustix::fs::{RenameFlags, renameat_with};
use rustix::io::Errno;

use crate::{
    CasOutcome, CompareExchangeError, DurabilityStep, ExactObject, ExpectedValue, ObjectKind,
    ObjectName, StorageContext, StorageError, StorageOperation, StorageResult,
};

use super::barrier::{sync_directory, sync_file, syscall};
use super::directory::DurableDirectory;
use super::metadata::inspect_private;
use super::objects;
use super::temporary;
use super::writer::WriterLease;

impl WriterLease {
    /// Replace a mutable file under the anchored writer lease.
    ///
    /// The call rejects an old value mismatch that it can observe. POSIX does
    /// not combine an exact byte comparison and a rename in one operation.
    pub fn compare_exchange_file(
        &self,
        directory: &DurableDirectory,
        name: &ObjectName,
        expected: ExpectedValue,
        new: ExactObject,
    ) -> StorageResult<CasOutcome> {
        match self.compare_exchange_file_guarded(directory, name, expected, new, || {
            Ok::<(), Infallible>(())
        }) {
            Ok(outcome) => Ok(outcome),
            Err(CompareExchangeError::Storage { source }) => Err(source),
            Err(CompareExchangeError::Validation { source }) => match source {},
            Err(CompareExchangeError::ValidationAndCleanup { validation, .. }) => {
                match validation {}
            }
        }
    }

    /// Replace a mutable file after one caller validation.
    ///
    /// The store calls `validate` after it makes the new temporary object and
    /// after an expected-value check. It checks the temporary object and the
    /// expected value again after caller validation. It does not call
    /// `validate` when the destination already has the exact new value.
    pub fn compare_exchange_file_guarded<E>(
        &self,
        directory: &DurableDirectory,
        name: &ObjectName,
        expected: ExpectedValue,
        new: ExactObject,
        validate: impl FnOnce() -> Result<(), E>,
    ) -> Result<CasOutcome, CompareExchangeError<E>> {
        compare_exchange_guarded(self, directory, name, expected, new, validate)
    }

    fn validate_expected_again(
        &self,
        directory: &DurableDirectory,
        name: &ObjectName,
        expected: &ExpectedValue,
        new: &ExactObject,
        context: &StorageContext,
    ) -> StorageResult<Recheck> {
        self.validate_for(&directory.handle, context)?;
        let current = read_current(directory, name, limit_for(expected, new), context)?;
        if expected_matches(current.as_ref(), expected) {
            return Ok(Recheck::Proceed);
        }
        if current.as_ref() == Some(new) {
            return Ok(Recheck::AlreadyExact);
        }
        Err(StorageError::StaleExpected {
            context: context.clone(),
        })
    }
}

fn compare_exchange_guarded<E>(
    lease: &WriterLease,
    directory: &DurableDirectory,
    name: &ObjectName,
    expected: ExpectedValue,
    new: ExactObject,
    validate: impl FnOnce() -> Result<(), E>,
) -> Result<CasOutcome, CompareExchangeError<E>> {
    temporary::reject_reserved_destination(
        directory,
        name,
        &new,
        StorageOperation::CompareExchange,
    )?;
    let before = context(directory, name, &new, DurabilityStep::BeforeMutation);
    lease.validate_for(&directory.handle, &before)?;
    match read_current(directory, name, limit_for(&expected, &new), &before)? {
        Some(current) if current == new => {
            stabilize_known_value(lease, directory, name, &new)?;
            return Ok(CasOutcome::AlreadyExact);
        }
        current if expected_matches(current.as_ref(), &expected) => {}
        _ => return Err(StorageError::StaleExpected { context: before }.into()),
    }
    let temporary = temporary::create(directory, lease, StorageOperation::CompareExchange, &new)?;
    match lease.validate_expected_again(directory, name, &expected, &new, &before) {
        Ok(Recheck::Proceed) => {}
        Ok(Recheck::AlreadyExact) => {
            cleanup_known_value(lease, directory, name, &new, &temporary)?;
            return Ok(CasOutcome::AlreadyExact);
        }
        Err(error) => return Err(error.into()),
    }
    temporary::verify_owned(directory, &temporary, StorageOperation::CompareExchange)?;
    if let Err(source) = validate() {
        return match lease.cleanup_owned_temporary(directory, &temporary) {
            Ok(()) => Err(CompareExchangeError::Validation { source }),
            Err(cleanup) => Err(CompareExchangeError::ValidationAndCleanup {
                validation: source,
                cleanup,
            }),
        };
    }
    temporary::verify_owned(directory, &temporary, StorageOperation::CompareExchange)?;
    match lease.validate_expected_again(directory, name, &expected, &new, &before) {
        Ok(Recheck::Proceed) => {}
        Ok(Recheck::AlreadyExact) => {
            cleanup_known_value(lease, directory, name, &new, &temporary)?;
            return Ok(CasOutcome::AlreadyExact);
        }
        Err(error) => return Err(error.into()),
    }
    exchange_prepared(lease, directory, name, &expected, &new, &temporary).map_err(Into::into)
}

fn cleanup_known_value(
    lease: &WriterLease,
    directory: &DurableDirectory,
    name: &ObjectName,
    new: &ExactObject,
    temporary: &crate::OwnedTemporary,
) -> StorageResult<()> {
    let readback = context(directory, name, new, DurabilityStep::AuthorizationReadback);
    lease
        .cleanup_owned_temporary(directory, temporary)
        .map_err(|source| unresolved(readback.clone(), source))?;
    stabilize_known_value(lease, directory, name, new)
}

fn stabilize_known_value(
    lease: &WriterLease,
    directory: &DurableDirectory,
    name: &ObjectName,
    new: &ExactObject,
) -> StorageResult<()> {
    let readback = context(directory, name, new, DurabilityStep::AuthorizationReadback);
    make_authorization_durable(lease, directory, name, new)
        .map_err(|source| unresolved(readback, source))
}

fn exchange_prepared(
    lease: &WriterLease,
    directory: &DurableDirectory,
    name: &ObjectName,
    expected: &ExpectedValue,
    new: &ExactObject,
    temporary: &crate::OwnedTemporary,
) -> StorageResult<CasOutcome> {
    let rename = context(directory, name, new, DurabilityStep::AuthorizationRename);
    let renamed = syscall(&directory.handle.anchor.faults, &rename, || {
        rename_authorization(directory, temporary.name(), name, expected)
    });
    if let Err(error) = renamed {
        return recover(lease, directory, name, expected, new, temporary, error);
    }
    let parent = context(directory, name, new, DurabilityStep::ParentDirectory);
    if let Err(error) = sync_directory(
        &directory.handle.fd,
        &directory.handle.anchor.faults,
        &parent,
    ) {
        return recover(lease, directory, name, expected, new, temporary, error);
    }
    validate_committed(lease, directory, name, new)?;
    Ok(CasOutcome::Exchanged)
}

fn validate_committed(
    lease: &WriterLease,
    directory: &DurableDirectory,
    name: &ObjectName,
    new: &ExactObject,
) -> StorageResult<()> {
    let readback = context(directory, name, new, DurabilityStep::AuthorizationReadback);
    directory
        .handle
        .anchor
        .faults
        .before(&readback)
        .and_then(|()| {
            objects::verify_exact(
                &directory.handle,
                name,
                new,
                StorageOperation::CompareExchange,
            )?;
            directory.handle.anchor.faults.after(&readback)
        })
        .map_err(|source| unresolved(readback, source))?;
    let after = context(directory, name, new, DurabilityStep::AfterMutation);
    lease
        .validate_for(&directory.handle, &after)
        .map_err(|source| unresolved(after, source))
}

fn rename_authorization(
    directory: &DurableDirectory,
    temporary: &ObjectName,
    destination: &ObjectName,
    expected: &ExpectedValue,
) -> rustix::io::Result<()> {
    if matches!(expected, ExpectedValue::Absent) {
        return rename_absent(directory, temporary, destination);
    }
    renameat(
        &directory.handle.fd,
        temporary.as_os_str(),
        &directory.handle.fd,
        destination.as_os_str(),
    )
}

#[cfg(any(target_vendor = "apple", target_os = "linux", target_os = "android"))]
pub(crate) fn rename_absent(
    directory: &DurableDirectory,
    temporary: &ObjectName,
    destination: &ObjectName,
) -> rustix::io::Result<()> {
    renameat_with(
        &directory.handle.fd,
        temporary.as_os_str(),
        &directory.handle.fd,
        destination.as_os_str(),
        RenameFlags::NOREPLACE,
    )
}

fn recover(
    lease: &WriterLease,
    directory: &DurableDirectory,
    name: &ObjectName,
    expected: &ExpectedValue,
    new: &ExactObject,
    temporary: &crate::OwnedTemporary,
    original: StorageError,
) -> StorageResult<CasOutcome> {
    let readback = context(directory, name, new, DurabilityStep::AuthorizationReadback);
    let recovered = directory
        .handle
        .anchor
        .faults
        .before(&readback)
        .and_then(|()| {
            let value = read_current(directory, name, limit_for(expected, new), &readback)?;
            directory.handle.anchor.faults.after(&readback)?;
            Ok(value)
        });
    let current = match recovered {
        Ok(value) => value,
        Err(error) => return Err(ambiguous(readback, Some(original), error)),
    };
    if current.as_ref() == Some(new) {
        let temporary_exists = match directory.exists(temporary.name()) {
            Ok(exists) => exists,
            Err(source) => return Err(ambiguous(readback, Some(original), source)),
        };
        if temporary_exists && let Err(source) = lease.cleanup_owned_temporary(directory, temporary)
        {
            return Err(ambiguous(readback, Some(original), source));
        }
        let barrier = context(directory, name, new, DurabilityStep::RecoveryBarrier);
        if let Err(error) = sync_directory(
            &directory.handle.fd,
            &directory.handle.anchor.faults,
            &barrier,
        ) {
            return Err(ambiguous(barrier, Some(original), error));
        }
        if let Err(source) = validate_committed(lease, directory, name, new) {
            return Err(ambiguous(barrier, Some(original), source));
        }
        return Ok(CasOutcome::AlreadyExact);
    }
    if expected_matches(current.as_ref(), expected) {
        if directory.exists(temporary.name())? {
            lease.cleanup_owned_temporary(directory, temporary)?;
        }
        return Err(original);
    }
    Err(StorageError::StaleExpected { context: readback })
}

fn read_current(
    directory: &DurableDirectory,
    name: &ObjectName,
    maximum_bytes: usize,
    context: &StorageContext,
) -> StorageResult<Option<ExactObject>> {
    let stat = match statat(
        &directory.handle.fd,
        name.as_os_str(),
        AtFlags::SYMLINK_NOFOLLOW,
    ) {
        Ok(stat) => stat,
        Err(Errno::NOENT) => return Ok(None),
        Err(source) => {
            return Err(StorageError::Io {
                context: context.clone(),
                source: source.into(),
            });
        }
    };
    let inspected = inspect_private(&stat, context)?;
    if inspected.kind != ObjectKind::RegularFile {
        return Err(StorageError::WrongType {
            context: context.clone(),
        });
    }
    if inspected.size > maximum_bytes as u64 {
        return Err(StorageError::Corruption {
            reason: "mutable object exceeds all exact values",
            context: context.clone(),
        });
    }
    objects::read_exact(&directory.handle, name, maximum_bytes).map(Some)
}

fn make_authorization_durable(
    lease: &WriterLease,
    directory: &DurableDirectory,
    name: &ObjectName,
    new: &ExactObject,
) -> StorageResult<()> {
    objects::verify_exact(
        &directory.handle,
        name,
        new,
        StorageOperation::CompareExchange,
    )?;
    let object = context(directory, name, new, DurabilityStep::ObjectData);
    let named = statat(
        &directory.handle.fd,
        name.as_os_str(),
        AtFlags::SYMLINK_NOFOLLOW,
    )
    .map_err(|source| StorageError::Io {
        context: object.clone(),
        source: source.into(),
    })?;
    let named_inspection = inspect_private(&named, &object)?;
    let fd = objects::open_regular(&directory.handle, name, &object)?;
    objects::verify_open_identity(&fd, named_inspection.identity, &object)?;
    if let Err(error) = sync_file(&fd, &directory.handle.anchor.faults, &object) {
        return Err(ambiguous(object, None, error));
    }
    let parent = context(directory, name, new, DurabilityStep::ParentDirectory);
    if let Err(error) = sync_directory(
        &directory.handle.fd,
        &directory.handle.anchor.faults,
        &parent,
    ) {
        return Err(ambiguous(parent, None, error));
    }
    objects::verify_exact(
        &directory.handle,
        name,
        new,
        StorageOperation::CompareExchange,
    )?;
    let after = context(directory, name, new, DurabilityStep::AfterMutation);
    lease.validate_for(&directory.handle, &after)
}

fn expected_matches(current: Option<&ExactObject>, expected: &ExpectedValue) -> bool {
    match (current, expected) {
        (None, ExpectedValue::Absent) => true,
        (Some(actual), ExpectedValue::Exact(required)) => actual == required,
        _ => false,
    }
}

enum Recheck {
    Proceed,
    AlreadyExact,
}

fn limit_for(expected: &ExpectedValue, new: &ExactObject) -> usize {
    match expected {
        ExpectedValue::Absent => new.bytes().len(),
        ExpectedValue::Exact(old) => old.bytes().len().max(new.bytes().len()),
    }
}

fn ambiguous(
    context: StorageContext,
    prior: Option<StorageError>,
    source: StorageError,
) -> StorageError {
    StorageError::AmbiguousCommit {
        context,
        prior: prior.map(Box::new),
        source: Box::new(source),
    }
}

fn unresolved(context: StorageContext, source: StorageError) -> StorageError {
    if matches!(source, StorageError::AmbiguousCommit { .. }) {
        source
    } else {
        ambiguous(context, None, source)
    }
}

fn context(
    directory: &DurableDirectory,
    name: &ObjectName,
    object: &ExactObject,
    step: DurabilityStep,
) -> StorageContext {
    directory.handle.context_with_object(
        Some(name),
        Some(object.digest()),
        StorageOperation::CompareExchange,
        step,
    )
}
