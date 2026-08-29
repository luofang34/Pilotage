//! Durable synchronization of every prepared exact direct command.
//!
//! The transport prepares a command and then enacts it. Between those two
//! points a datagram can leave the process, so a run that stops there has
//! commanded the vehicle with nothing on disk that says so. The ledger
//! closes that window: the prepared intent is durable before enactment can
//! send, and the send result is durable after it returns.
//!
//! Recovery therefore reads one of three states, and the third is the one
//! that matters: a durable prepared intent with no durable send result is
//! ambiguous. The command may or may not have reached the flight
//! controller. The ledger reports that as its own outcome rather than
//! guessing, because both guesses are wrong for some run.

use std::path::Path;

use flight_tune::Digest;
use pilotage_durable_storage::{
    DurableDirectory, DurableStore, ExactObject, ObjectName, PutOutcome, WriterLease,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

use crate::direct_transport::{
    DirectCommandPurpose, DirectCommandRecord, DirectEnactment, DirectSetpoint,
    PreparedDirectCommand,
};
use crate::runtime::AviateRuntimeError;

/// The supported direct intent and result document schema.
pub const DIRECT_INTENT_SCHEMA_VERSION: u16 = 1;

/// The largest direct ledger document this runtime writes or reads.
const MAX_LEDGER_DOCUMENT_BYTES: usize = 64 * 1024;

/// One prepared direct command, durable before a datagram can leave.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DirectIntentRecord {
    /// Document schema version.
    pub schema_version: u16,
    /// The zero-based direct command sequence inside one run.
    pub sequence: u64,
    /// What the prepared command is for.
    pub purpose: DirectCommandPurpose,
    /// The run intent that the command binds to.
    pub run_intent_digest: Digest,
    /// The direct transport that prepared the command.
    pub transport_identity_digest: Digest,
    /// The frozen stimulus envelope of the command.
    pub envelope_digest: Digest,
    /// The physical target the command will transmit.
    pub requested: DirectSetpoint,
}

/// What one durably prepared direct command resolved to.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "outcome", rename_all = "snake_case", deny_unknown_fields)]
pub enum DirectSendOutcome {
    /// The command reached the flight controller with a complete record.
    Enacted {
        /// The complete causal record of the command.
        record: Box<DirectCommandRecord>,
    },
    /// The raw source had not reached the command time. Nothing was sent.
    Pending {},
    /// The raw source carried no exact sample. Nothing was sent.
    NoExactSource {},
}

/// The durable result of one prepared direct command.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DirectSendResult {
    /// Document schema version.
    pub schema_version: u16,
    /// The direct command sequence this result closes.
    pub sequence: u64,
    /// The run intent that the command bound to.
    pub run_intent_digest: Digest,
    /// What the command resolved to.
    pub outcome: DirectSendOutcome,
}

/// What recovery can say about the last prepared direct command.
#[derive(Clone, Debug, PartialEq)]
pub enum DirectRecoveryOutcome {
    /// The run prepared no direct command that is still open.
    Idle,
    /// The last prepared command has a durable result.
    Resolved(Box<DirectSendResult>),
    /// A prepared command has no durable result.
    ///
    /// The command may or may not have reached the flight controller. A
    /// run that reaches this state cannot be scored, and the campaign must
    /// quarantine it rather than resume it.
    Ambiguous(Box<DirectIntentRecord>),
}

/// The durable store one direct ledger writes through.
pub trait DirectIntentStore {
    /// Makes one prepared direct command durable.
    ///
    /// # Errors
    ///
    /// Returns [`AviateRuntimeError`] when the document cannot be published
    /// or does not read back unchanged.
    fn publish_intent(&mut self, record: &DirectIntentRecord)
    -> Result<Digest, AviateRuntimeError>;

    /// Makes the result of one prepared direct command durable.
    ///
    /// # Errors
    ///
    /// Returns [`AviateRuntimeError`] when the document cannot be published
    /// or does not read back unchanged.
    fn publish_result(&mut self, result: &DirectSendResult) -> Result<Digest, AviateRuntimeError>;

    /// Reads the state of the direct command at one sequence.
    ///
    /// # Errors
    ///
    /// Returns [`AviateRuntimeError`] when a stored document is unreadable.
    fn read_state(&self, sequence: u64) -> Result<DirectRecoveryOutcome, AviateRuntimeError>;
}

/// The durable direct command ledger of one run.
#[derive(Clone, Copy, Debug, Default)]
pub struct DirectIntentLedger {
    sequence: u64,
    open: bool,
}

impl DirectIntentLedger {
    /// Creates one ledger with no prepared command.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            sequence: 0,
            open: false,
        }
    }

    /// The next direct command sequence this run will use.
    #[must_use]
    pub const fn sequence(&self) -> u64 {
        self.sequence
    }

    /// Whether one prepared command has no durable result yet.
    #[must_use]
    pub const fn is_open(&self) -> bool {
        self.open
    }

    /// Makes one prepared command durable before enactment can send.
    ///
    /// # Errors
    ///
    /// Returns [`AviateRuntimeError`] when a prepared command is already
    /// open, or when the document cannot be made durable.
    pub fn prepare<S: DirectIntentStore + ?Sized>(
        &mut self,
        store: &mut S,
        prepared: &PreparedDirectCommand,
    ) -> Result<DirectIntentRecord, AviateRuntimeError> {
        if self.open {
            return Err(AviateRuntimeError::DirectIntentOpen {
                sequence: self.sequence,
            });
        }
        let record = DirectIntentRecord {
            schema_version: DIRECT_INTENT_SCHEMA_VERSION,
            sequence: self.sequence,
            purpose: prepared.purpose(),
            run_intent_digest: prepared.run_intent_digest(),
            transport_identity_digest: prepared.transport_identity_digest(),
            envelope_digest: prepared.envelope_digest(),
            requested: prepared.requested(),
        };
        store.publish_intent(&record)?;
        self.open = true;
        Ok(record)
    }

    /// Makes the result of the open prepared command durable.
    ///
    /// # Errors
    ///
    /// Returns [`AviateRuntimeError`] when no prepared command is open, or
    /// when the document cannot be made durable.
    pub fn resolve<S: DirectIntentStore + ?Sized>(
        &mut self,
        store: &mut S,
        intent: &DirectIntentRecord,
        enactment: &DirectEnactment,
    ) -> Result<DirectSendResult, AviateRuntimeError> {
        if !self.open || intent.sequence != self.sequence {
            return Err(AviateRuntimeError::NoOpenDirectIntent);
        }
        let result = DirectSendResult {
            schema_version: DIRECT_INTENT_SCHEMA_VERSION,
            sequence: intent.sequence,
            run_intent_digest: intent.run_intent_digest,
            outcome: outcome_of(intent, enactment)?,
        };
        store.publish_result(&result)?;
        self.open = false;
        self.sequence = self.sequence.wrapping_add(1);
        Ok(result)
    }
}

fn outcome_of(
    intent: &DirectIntentRecord,
    enactment: &DirectEnactment,
) -> Result<DirectSendOutcome, AviateRuntimeError> {
    Ok(match enactment {
        DirectEnactment::Enacted(record) => {
            if record.run_intent_digest != intent.run_intent_digest
                || record.requested != intent.requested
                || record.envelope_digest != intent.envelope_digest
                || record.transport_identity_digest != intent.transport_identity_digest
            {
                return Err(AviateRuntimeError::DirectRecordMismatch);
            }
            DirectSendOutcome::Enacted {
                record: record.clone(),
            }
        }
        DirectEnactment::Pending => DirectSendOutcome::Pending {},
        DirectEnactment::NoExactSource => DirectSendOutcome::NoExactSource {},
    })
}

/// The direct ledger of one run, on crash-durable private storage.
pub struct DurableDirectIntentStore {
    directory: DurableDirectory,
    writer: WriterLease,
}

impl DurableDirectIntentStore {
    /// Opens or creates the direct ledger under one private root.
    ///
    /// # Errors
    ///
    /// Returns [`AviateRuntimeError`] when the root cannot be opened or the
    /// writer lease cannot be acquired.
    pub fn open_blocking(root: &Path) -> Result<Self, AviateRuntimeError> {
        let store = DurableStore::open_or_create(root)
            .map_err(|source| storage_error("open the direct ledger root", source))?;
        let writer = store
            .acquire_writer()
            .map_err(|source| storage_error("acquire the direct ledger writer lease", source))?;
        let directory = store.root_directory();
        writer
            .validate(&directory)
            .map_err(|source| storage_error("validate the direct ledger writer lease", source))?;
        Ok(Self { directory, writer })
    }

    fn publish<T: Serialize>(&self, name: &str, value: &T) -> Result<Digest, AviateRuntimeError> {
        let bytes = serde_json::to_vec(value).map_err(|source| AviateRuntimeError::Encode {
            document: "direct ledger",
            source,
        })?;
        if bytes.len() > MAX_LEDGER_DOCUMENT_BYTES {
            return Err(AviateRuntimeError::LedgerDocumentSize { bytes: bytes.len() });
        }
        let object_name = object_name(name)?;
        match self
            .directory
            .put_immutable_no_replace(
                &self.writer,
                &object_name,
                &ExactObject::from_bytes(bytes.clone()),
            )
            .map_err(|source| storage_error("publish a direct ledger document", source))?
        {
            PutOutcome::Published => {}
            PutOutcome::AlreadyExact => {
                return Err(AviateRuntimeError::DirectLedgerResidual {
                    name: name.to_owned(),
                });
            }
        }
        let readback = self
            .directory
            .read_exact(&object_name, MAX_LEDGER_DOCUMENT_BYTES)
            .map_err(|source| storage_error("read back a direct ledger document", source))?;
        if readback.bytes() != bytes {
            return Err(AviateRuntimeError::DirectLedgerReadback {
                name: name.to_owned(),
            });
        }
        Ok(digest_bytes(&bytes))
    }

    fn read<T: serde::de::DeserializeOwned>(
        &self,
        name: &str,
    ) -> Result<Option<T>, AviateRuntimeError> {
        let object = match self
            .directory
            .read_exact(&object_name(name)?, MAX_LEDGER_DOCUMENT_BYTES)
        {
            Ok(object) => object,
            Err(source) if is_absent(&source) => return Ok(None),
            Err(source) => {
                return Err(storage_error("read a direct ledger document", source));
            }
        };
        serde_json::from_slice(object.bytes())
            .map(Some)
            .map_err(|source| AviateRuntimeError::Decode {
                document: "direct ledger",
                source,
            })
    }
}

impl DirectIntentStore for DurableDirectIntentStore {
    fn publish_intent(
        &mut self,
        record: &DirectIntentRecord,
    ) -> Result<Digest, AviateRuntimeError> {
        self.publish(&intent_name(record.sequence), record)
    }

    fn publish_result(&mut self, result: &DirectSendResult) -> Result<Digest, AviateRuntimeError> {
        self.publish(&result_name(result.sequence), result)
    }

    fn read_state(&self, sequence: u64) -> Result<DirectRecoveryOutcome, AviateRuntimeError> {
        let Some(intent) = self.read::<DirectIntentRecord>(&intent_name(sequence))? else {
            return Ok(DirectRecoveryOutcome::Idle);
        };
        match self.read::<DirectSendResult>(&result_name(sequence))? {
            Some(result) => Ok(DirectRecoveryOutcome::Resolved(Box::new(result))),
            None => Ok(DirectRecoveryOutcome::Ambiguous(Box::new(intent))),
        }
    }
}

fn intent_name(sequence: u64) -> String {
    format!("direct-intent-{sequence:016x}")
}

fn result_name(sequence: u64) -> String {
    format!("direct-result-{sequence:016x}")
}

fn object_name(name: &str) -> Result<ObjectName, AviateRuntimeError> {
    ObjectName::new(name).map_err(|source| storage_error("select a direct ledger document", source))
}

/// Whether one storage failure means the document simply does not exist.
///
/// A ledger read that finds nothing is the normal state before the run
/// prepares its first direct command. Every other failure is a real one.
fn is_absent(error: &pilotage_durable_storage::StorageError) -> bool {
    matches!(
        error,
        pilotage_durable_storage::StorageError::Io { source, .. }
            if source.kind() == std::io::ErrorKind::NotFound
    )
}

fn digest_bytes(bytes: &[u8]) -> Digest {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    Digest::from_bytes(hasher.finalize().into())
}

fn storage_error(
    operation: &'static str,
    source: pilotage_durable_storage::StorageError,
) -> AviateRuntimeError {
    AviateRuntimeError::Storage {
        operation,
        source: Box::new(source),
    }
}
