use std::collections::HashSet;
use std::fs::{self, File, OpenOptions, TryLockError};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize, de::DeserializeOwned};

use crate::identity::digest_bytes;
use crate::journal::JournalEntry;
use crate::{Candidate, Digest, SearchStage, TuneError};

#[cfg(test)]
#[path = "storage/tests.rs"]
mod tests;

const MAX_DOCUMENT_BYTES: u64 = 8 * 1024 * 1024;
const MAX_CHAIN_ENTRIES: usize = 100_000;
const TEMPORARY_ATTEMPTS: u32 = 1_024;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct HeadPointer {
    digest: Digest,
}

pub(super) struct WriterLock {
    _file: File,
}

pub(super) fn ensure_layout(root: &Path) -> Result<(), TuneError> {
    create_directory(root)?;
    create_directory(&candidate_directory(root))?;
    create_directory(&stage_directory(root))?;
    create_directory(&entry_directory(root))
}

pub(super) fn acquire_writer_lock(root: &Path) -> Result<WriterLock, TuneError> {
    let path = root.join("WRITER.lock");
    let file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(&path)
        .map_err(|source| io_error("open writer lock", &path, source))?;
    match file.try_lock() {
        Ok(()) => Ok(WriterLock { _file: file }),
        Err(TryLockError::WouldBlock) => Err(TuneError::JournalLocked {
            path: root.to_path_buf(),
        }),
        Err(TryLockError::Error(source)) => Err(io_error("lock journal writer", &path, source)),
    }
}

pub(super) fn head_exists(root: &Path) -> Result<bool, TuneError> {
    let path = head_path(root);
    path.try_exists()
        .map_err(|source| io_error("inspect journal head", &path, source))
}

pub(super) fn document_digest<T: Serialize>(
    document: &'static str,
    value: &T,
) -> Result<Digest, TuneError> {
    let bytes = encode(document, value)?;
    Ok(digest_bytes(&bytes))
}

pub(super) fn store_candidate(root: &Path, candidate: &Candidate) -> Result<Digest, TuneError> {
    let bytes = encode("candidate", candidate)?;
    let digest = digest_bytes(&bytes);
    write_immutable(&candidate_path(root, digest), &bytes, digest)?;
    Ok(digest)
}

pub(super) fn read_candidate(root: &Path, digest: Digest) -> Result<Candidate, TuneError> {
    let path = candidate_path(root, digest);
    let bytes = read_verified(&path, digest)?;
    let candidate: Candidate = decode("candidate", &path, &bytes)?;
    candidate.validate()?;
    Ok(candidate)
}

pub(super) fn store_stage(root: &Path, stage: &SearchStage) -> Result<Digest, TuneError> {
    let bytes = encode("search stage", stage)?;
    let digest = digest_bytes(&bytes);
    write_immutable(&stage_path(root, digest), &bytes, digest)?;
    Ok(digest)
}

pub(super) fn read_stage(root: &Path, digest: Digest) -> Result<SearchStage, TuneError> {
    let path = stage_path(root, digest);
    let bytes = read_verified(&path, digest)?;
    let stage: SearchStage = decode("search stage", &path, &bytes)?;
    stage.validate()?;
    Ok(stage)
}

pub(super) fn append_entry(root: &Path, entry: &JournalEntry) -> Result<Digest, TuneError> {
    let bytes = encode("journal entry", entry)?;
    let digest = digest_bytes(&bytes);
    write_immutable(&entry_path(root, digest), &bytes, digest)?;
    let head = encode("journal head", &HeadPointer { digest })?;
    atomic_replace(&head_path(root), &head)?;
    Ok(digest)
}

pub(super) fn load_entries(root: &Path) -> Result<Vec<(Digest, JournalEntry)>, TuneError> {
    let head_file = head_path(root);
    let head_bytes = read_limited(&head_file)?;
    let head: HeadPointer = decode("journal head", &head_file, &head_bytes)?;
    let mut entries = Vec::new();
    let mut seen = HashSet::new();
    let mut next = Some(head.digest);
    while let Some(digest) = next {
        if entries.len() >= MAX_CHAIN_ENTRIES || !seen.insert(digest) {
            return Err(invalid_journal(
                "the journal chain is too long or has a cycle",
            ));
        }
        let path = entry_path(root, digest);
        let bytes = read_verified(&path, digest)?;
        let entry: JournalEntry = decode("journal entry", &path, &bytes)?;
        next = entry.previous;
        entries.push((digest, entry));
    }
    entries.reverse();
    Ok(entries)
}

fn create_directory(path: &Path) -> Result<(), TuneError> {
    fs::create_dir_all(path).map_err(|source| io_error("create directory", path, source))
}

fn write_immutable(path: &Path, bytes: &[u8], digest: Digest) -> Result<(), TuneError> {
    if path
        .try_exists()
        .map_err(|source| io_error("inspect immutable object", path, source))?
    {
        let existing = read_limited(path)?;
        if existing == bytes {
            return Ok(());
        }
        return Err(TuneError::DigestMismatch { expected: digest });
    }
    atomic_replace(path, bytes)
}

fn atomic_replace(path: &Path, bytes: &[u8]) -> Result<(), TuneError> {
    check_size(path.display().to_string(), bytes.len())?;
    let (mut file, temporary) = create_unique_temporary(path)?;
    file.write_all(bytes)
        .map_err(|source| io_error("write temporary file", &temporary, source))?;
    file.sync_all()
        .map_err(|source| io_error("synchronize temporary file", &temporary, source))?;
    fs::rename(&temporary, path).map_err(|source| io_error("replace file", path, source))?;
    synchronize_parent(path)
}

fn create_unique_temporary(path: &Path) -> Result<(File, PathBuf), TuneError> {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|source| TuneError::InvalidJournal {
            detail: format!("system time is before the Unix epoch: {source}"),
        })?
        .as_nanos();
    let mut attempt = 0_u32;
    while attempt < TEMPORARY_ATTEMPTS {
        let temporary = temporary_path(path, timestamp, attempt);
        match OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)
        {
            Ok(file) => return Ok((file, temporary)),
            Err(source) if source.kind() == std::io::ErrorKind::AlreadyExists => {
                attempt = attempt.wrapping_add(1);
            }
            Err(source) => return Err(io_error("create temporary file", &temporary, source)),
        }
    }
    Err(invalid_journal("cannot allocate a unique temporary file"))
}

fn synchronize_parent(path: &Path) -> Result<(), TuneError> {
    let parent = path
        .parent()
        .ok_or_else(|| invalid_journal("an atomic file has no parent directory"))?;
    let directory =
        File::open(parent).map_err(|source| io_error("open parent directory", parent, source))?;
    directory
        .sync_all()
        .map_err(|source| io_error("synchronize parent directory", parent, source))
}

fn read_verified(path: &Path, expected: Digest) -> Result<Vec<u8>, TuneError> {
    let bytes = read_limited(path)?;
    if digest_bytes(&bytes) != expected {
        return Err(TuneError::DigestMismatch { expected });
    }
    Ok(bytes)
}

fn read_limited(path: &Path) -> Result<Vec<u8>, TuneError> {
    let metadata = fs::metadata(path).map_err(|source| io_error("inspect file", path, source))?;
    check_size(path.display().to_string(), metadata.len() as usize)?;
    let mut file = File::open(path).map_err(|source| io_error("open file", path, source))?;
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.read_to_end(&mut bytes)
        .map_err(|source| io_error("read file", path, source))?;
    Ok(bytes)
}

fn encode<T: Serialize>(document: &'static str, value: &T) -> Result<Vec<u8>, TuneError> {
    let bytes =
        serde_json::to_vec(value).map_err(|source| TuneError::Encode { document, source })?;
    check_size(document.to_owned(), bytes.len())?;
    Ok(bytes)
}

fn decode<T: DeserializeOwned>(
    document: &'static str,
    path: &Path,
    bytes: &[u8],
) -> Result<T, TuneError> {
    serde_json::from_slice(bytes).map_err(|source| TuneError::Decode {
        document,
        path: path.to_path_buf(),
        source,
    })
}

fn check_size(document: String, size: usize) -> Result<(), TuneError> {
    let size = u64::try_from(size).unwrap_or(u64::MAX);
    if size > MAX_DOCUMENT_BYTES {
        return Err(TuneError::DocumentTooLarge {
            document,
            size,
            limit: MAX_DOCUMENT_BYTES,
        });
    }
    Ok(())
}

fn candidate_directory(root: &Path) -> PathBuf {
    root.join("candidates")
}

fn entry_directory(root: &Path) -> PathBuf {
    root.join("entries")
}

fn stage_directory(root: &Path) -> PathBuf {
    root.join("stages")
}

fn candidate_path(root: &Path, digest: Digest) -> PathBuf {
    candidate_directory(root).join(format!("{digest}.json"))
}

fn entry_path(root: &Path, digest: Digest) -> PathBuf {
    entry_directory(root).join(format!("{digest}.json"))
}

fn stage_path(root: &Path, digest: Digest) -> PathBuf {
    stage_directory(root).join(format!("{digest}.json"))
}

fn head_path(root: &Path) -> PathBuf {
    root.join("HEAD.json")
}

fn temporary_path(path: &Path, timestamp: u128, attempt: u32) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(format!(
        ".next.{}.{}.{}",
        std::process::id(),
        timestamp,
        attempt
    ));
    PathBuf::from(name)
}

fn io_error(operation: &'static str, path: &Path, source: std::io::Error) -> TuneError {
    TuneError::Io {
        operation,
        path: path.to_path_buf(),
        source,
    }
}

fn invalid_journal(detail: impl Into<String>) -> TuneError {
    TuneError::InvalidJournal {
        detail: detail.into(),
    }
}
