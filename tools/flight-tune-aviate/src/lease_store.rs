use std::path::Path;

use pilotage_durable_storage::{
    DurableDirectory, DurableStore, ExactObject, ObjectName, PutOutcome, WriterLease,
};
use serde::{Serialize, de::DeserializeOwned};
use sha2::{Digest as _, Sha256};

use crate::AviateSupervisorError;

const MAX_DOCUMENT_BYTES: usize = 64 * 1024;
const MAX_RECOVERY_TEMPORARIES: usize = 16;
const WRITER_LOCK_NAME: &str = ".pilotage-writer-lock";

pub(crate) struct LeaseStore {
    directory: DurableDirectory,
    writer: WriterLease,
}

impl LeaseStore {
    pub(crate) fn create_fresh(root: &Path) -> Result<Self, AviateSupervisorError> {
        let store = open_store(root)?;
        Self::bind_fresh(store)
    }

    #[cfg(test)]
    pub(crate) fn create_fresh_with_faults(
        root: &Path,
        faults: pilotage_durable_storage::FaultController,
    ) -> Result<Self, AviateSupervisorError> {
        let store = DurableStore::open_or_create_with_faults(root, faults)
            .map_err(|source| storage_error("open faulted process lease root", source))?;
        Self::bind_fresh(store)
    }

    fn bind_fresh(store: DurableStore) -> Result<Self, AviateSupervisorError> {
        let writer = acquire_writer(&store)?;
        let directory = store.root_directory();
        writer
            .validate(&directory)
            .map_err(|source| storage_error("validate new writer lease", source))?;
        let entries = directory
            .list()
            .map_err(|source| storage_error("scan new process lease", source))?;
        let is_fresh =
            entries.len() == 1 && entries[0].as_os_str() == std::ffi::OsStr::new(WRITER_LOCK_NAME);
        if !is_fresh {
            return Err(AviateSupervisorError::invalid_document(
                "process lease",
                "the new lease root contains residual process artifacts",
            ));
        }
        Ok(Self { directory, writer })
    }

    pub(crate) fn open_existing(root: &Path) -> Result<Self, AviateSupervisorError> {
        let store = open_existing_store(root)?;
        let writer = store.acquire_writer().map_err(|source| {
            if source.is_writer_locked() {
                AviateSupervisorError::SupervisorActive
            } else {
                storage_error("acquire recovery writer lease", source)
            }
        })?;
        let directory = store.root_directory();
        writer
            .validate(&directory)
            .map_err(|source| storage_error("validate recovery writer lease", source))?;
        Ok(Self { directory, writer })
    }

    pub(crate) fn publish<T: Serialize>(
        &self,
        name: &'static str,
        value: &T,
    ) -> Result<flight_tune::Digest, AviateSupervisorError> {
        let bytes = serde_json::to_vec(value).map_err(|source| {
            AviateSupervisorError::invalid_json_document(name, "encoding", source)
        })?;
        if bytes.len() > MAX_DOCUMENT_BYTES {
            return Err(AviateSupervisorError::invalid_document(
                name,
                "the encoded document exceeds its byte limit",
            ));
        }
        let object_name = object_name(name)?;
        let object = ExactObject::from_bytes(bytes.clone());
        match self
            .directory
            .put_immutable_no_replace(&self.writer, &object_name, &object)
            .map_err(|source| storage_error("publish process document", source))?
        {
            PutOutcome::Published => {}
            PutOutcome::AlreadyExact => {
                return Err(AviateSupervisorError::invalid_document(
                    name,
                    "a residual exact document already exists",
                ));
            }
        }
        let readback = self
            .directory
            .read_exact(&object_name, MAX_DOCUMENT_BYTES)
            .map_err(|source| storage_error("read back process document", source))?;
        if readback.bytes() != bytes {
            return Err(AviateSupervisorError::invalid_document(
                name,
                "durable readback changed the document bytes",
            ));
        }
        Ok(digest_bytes(&bytes))
    }

    pub(crate) fn read<T: DeserializeOwned + Serialize>(
        &self,
        name: &'static str,
    ) -> Result<(T, flight_tune::Digest), AviateSupervisorError> {
        let object_name = object_name(name)?;
        let object = self
            .directory
            .read_exact(&object_name, MAX_DOCUMENT_BYTES)
            .map_err(|source| storage_error("read process document", source))?;
        decode_document(name, &object)
    }

    pub(crate) fn repair<T: DeserializeOwned + Serialize>(
        &self,
        name: &'static str,
    ) -> Result<(T, flight_tune::Digest), AviateSupervisorError> {
        self.repair_optional(name)?.ok_or_else(|| {
            AviateSupervisorError::invalid_document(name, "the required document is absent")
        })
    }

    pub(crate) fn repair_optional<T: DeserializeOwned + Serialize>(
        &self,
        name: &'static str,
    ) -> Result<Option<(T, flight_tune::Digest)>, AviateSupervisorError> {
        let object = self
            .directory
            .repair_immutable_publication_blocking(
                &self.writer,
                &object_name(name)?,
                MAX_DOCUMENT_BYTES,
            )
            .map_err(|source| storage_error("repair process document", source))?;
        object
            .as_ref()
            .map(|object| decode_document(name, object))
            .transpose()
    }

    pub(crate) fn finish_recovery_scan(
        &self,
        document_names: &[&'static str],
    ) -> Result<(), AviateSupervisorError> {
        self.directory
            .cleanup_unlinked_temporaries_blocking(
                &self.writer,
                MAX_RECOVERY_TEMPORARIES,
                MAX_DOCUMENT_BYTES,
            )
            .map_err(|source| storage_error("clean recovery temporary objects", source))?;
        let entries = self
            .directory
            .list()
            .map_err(|source| storage_error("validate recovery document set", source))?;
        let valid = entries.iter().all(|entry| {
            entry.as_os_str() == std::ffi::OsStr::new(WRITER_LOCK_NAME)
                || document_names
                    .iter()
                    .any(|name| entry.as_os_str() == std::ffi::OsStr::new(name))
        });
        if !valid {
            return Err(AviateSupervisorError::invalid_document(
                "process lease",
                "the recovery document set contains an unknown object",
            ));
        }
        self.writer
            .validate(&self.directory)
            .map_err(|source| storage_error("validate recovery document set", source))
    }
}

pub(crate) fn read_without_writer<T: DeserializeOwned + Serialize>(
    root: &Path,
    name: &'static str,
) -> Result<(T, flight_tune::Digest), AviateSupervisorError> {
    let store = open_existing_store(root)?;
    let directory = store.root_directory();
    let object = directory
        .read_exact(&object_name(name)?, MAX_DOCUMENT_BYTES)
        .map_err(|source| storage_error("read active process document", source))?;
    decode_document(name, &object)
}

pub(crate) fn digest_bytes(bytes: &[u8]) -> flight_tune::Digest {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    flight_tune::Digest::from_bytes(hasher.finalize().into())
}

fn decode_document<T: DeserializeOwned + Serialize>(
    name: &'static str,
    object: &ExactObject,
) -> Result<(T, flight_tune::Digest), AviateSupervisorError> {
    let value = serde_json::from_slice(object.bytes())
        .map_err(|source| AviateSupervisorError::invalid_json_document(name, "parsing", source))?;
    let canonical = serde_json::to_vec(&value).map_err(|source| {
        AviateSupervisorError::invalid_json_document(name, "re-encoding", source)
    })?;
    if canonical != object.bytes() {
        return Err(AviateSupervisorError::invalid_document(
            name,
            "the document is not canonical JSON",
        ));
    }
    Ok((value, digest_bytes(&canonical)))
}

fn open_store(root: &Path) -> Result<DurableStore, AviateSupervisorError> {
    DurableStore::open_or_create(root)
        .map_err(|source| storage_error("open process lease root", source))
}

fn open_existing_store(root: &Path) -> Result<DurableStore, AviateSupervisorError> {
    DurableStore::open_existing_blocking(root)
        .map_err(|source| storage_error("open existing process lease root", source))
}

fn acquire_writer(store: &DurableStore) -> Result<WriterLease, AviateSupervisorError> {
    store
        .acquire_writer()
        .map_err(|source| storage_error("acquire process writer lease", source))
}

fn object_name(name: &'static str) -> Result<ObjectName, AviateSupervisorError> {
    ObjectName::new(name).map_err(|source| storage_error("select process document", source))
}

fn storage_error(
    operation: &'static str,
    source: pilotage_durable_storage::StorageError,
) -> AviateSupervisorError {
    AviateSupervisorError::Storage {
        operation,
        source: Box::new(source),
    }
}
