mod anchor;
mod barrier;
mod cas;
mod directory;
mod metadata;
mod objects;
mod remove;
mod store;
mod temporary;
mod writer;

#[cfg(test)]
pub(crate) use cas::rename_absent as rename_absent_for_test;
pub use directory::DurableDirectory;
#[cfg(test)]
pub(crate) use remove::validate_absent_after;

#[cfg(test)]
pub(crate) fn regular_open_flags_for_test(
    directory: &DurableDirectory,
    name: &crate::ObjectName,
) -> crate::StorageResult<rustix::fs::OFlags> {
    let context = directory.handle.context(
        Some(name),
        crate::StorageOperation::ReadObject,
        crate::DurabilityStep::ObjectReadback,
    );
    let fd = objects::open_regular(&directory.handle, name, &context)?;
    rustix::fs::fcntl_getfl(&fd).map_err(|source| crate::StorageError::Io {
        context,
        source: source.into(),
    })
}

pub use store::DurableStore;
pub use writer::WriterLease;
