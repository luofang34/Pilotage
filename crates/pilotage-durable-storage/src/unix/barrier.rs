use std::io;
use std::os::fd::AsFd;

use rustix::fs::fsync;

use crate::fault::FaultController;
use crate::{StorageContext, StorageError, StorageResult};

pub(crate) fn syscall<T>(
    faults: &FaultController,
    context: &StorageContext,
    call: impl FnOnce() -> rustix::io::Result<T>,
) -> StorageResult<T> {
    faults.before(context)?;
    let value = call().map_err(|source| StorageError::Io {
        context: context.clone(),
        source: io::Error::from(source),
    })?;
    faults.after(context)?;
    Ok(value)
}

pub(crate) fn sync_file(
    fd: impl AsFd,
    faults: &FaultController,
    context: &StorageContext,
) -> StorageResult<()> {
    syscall(faults, context, || {
        fsync(fd.as_fd())?;
        #[cfg(target_vendor = "apple")]
        rustix::fs::fcntl_fullfsync(fd.as_fd())?;
        Ok(())
    })
}

pub(crate) fn sync_directory(
    fd: impl AsFd,
    faults: &FaultController,
    context: &StorageContext,
) -> StorageResult<()> {
    syscall(faults, context, || fsync(fd.as_fd()))
}
