use std::path::Path;

use flight_tune::Digest;
use pilotage_durable_storage::{DurableStore, ExactObject, ObjectName};
use serde::Serialize;

use crate::evidence::EvidenceReceipt;
use crate::{FeedbackError, digest, error::invalid};

const MAX_EVIDENCE_BYTES: usize = 32 * 1024 * 1024;

pub(crate) fn store_blocking<T: Serialize>(
    value: &T,
    root: &Path,
) -> Result<EvidenceReceipt, FeedbackError> {
    let bytes = digest::encode("campaign evidence", value)?;
    let identity = digest::hash(&bytes);
    let store = DurableStore::open_or_create(root).map_err(durable)?;
    let writer = store.acquire_writer().map_err(durable)?;
    let directory_name = ObjectName::new("objects").map_err(durable)?;
    let directory = store
        .root_directory()
        .child(&writer, &directory_name)
        .map_err(durable)?;
    let file_name = format!("{identity}.json");
    let object_name = ObjectName::new(&file_name).map_err(durable)?;
    let object = ExactObject::from_bytes(bytes);
    directory
        .put_immutable_no_replace(&writer, &object_name, &object)
        .map_err(durable)?;
    writer.validate(&directory).map_err(durable)?;
    let object_path = root.join("objects").join(file_name);
    Ok(EvidenceReceipt {
        digest: identity,
        object_path,
    })
}

pub(crate) fn load_blocking(path: &Path) -> Result<Vec<u8>, FeedbackError> {
    let file_name = path
        .file_name()
        .ok_or_else(|| invalid("the evidence path has no object name"))?;
    let directory_path = path
        .parent()
        .filter(|parent| parent.file_name().is_some_and(|name| name == "objects"))
        .ok_or_else(|| invalid("the evidence object is not in an objects directory"))?;
    let root = directory_path
        .parent()
        .ok_or_else(|| invalid("the evidence object has no storage root"))?;
    let store = DurableStore::open_existing_blocking(root).map_err(durable)?;
    let writer = store.acquire_writer().map_err(durable)?;
    let directory_name = ObjectName::new("objects").map_err(durable)?;
    let directory = store
        .root_directory()
        .child(&writer, &directory_name)
        .map_err(durable)?;
    let object_name = ObjectName::new(file_name).map_err(durable)?;
    let object = directory
        .read_exact(&object_name, MAX_EVIDENCE_BYTES)
        .map_err(durable)?;
    writer.validate(&directory).map_err(durable)?;
    Ok(object.bytes().to_vec())
}

pub(crate) fn require_name(path: &Path, digest: Digest) -> Result<(), FeedbackError> {
    let expected = digest.to_string();
    if path.file_stem().and_then(|name| name.to_str()) != Some(expected.as_str()) {
        return Err(invalid(
            "the content-addressed evidence name does not match its digest",
        ));
    }
    Ok(())
}

fn durable(source: pilotage_durable_storage::StorageError) -> FeedbackError {
    FeedbackError::DurableStorage {
        source: Box::new(source),
    }
}
