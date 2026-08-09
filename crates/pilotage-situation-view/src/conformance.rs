//! Shared deterministic corpus for every SituationView implementation.

use std::collections::BTreeMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::{
    ComposingSituationViewV1, DomainSnapshotV1, MissingDataReasonV1, SITUATION_VIEW_CORPUS_VERSION,
    SituationViewError, SituationViewRequestV1, SituationViewResultV1, SituationViewV1,
    SnapshotCaptureV1, SnapshotSourceV1, TimeQueryV1,
};

const CORPUS_JSON: &str = include_str!("../corpus/situation-view-v1.json");

/// One domain capture in a corpus case.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "state")]
pub enum CorpusCaptureV1 {
    /// An immutable domain snapshot is available.
    Available {
        /// Captured snapshot value.
        snapshot: DomainSnapshotV1,
    },
    /// The selected domain snapshot is not available.
    Missing {
        /// Stable domain name.
        domain: String,
        /// Domain-owned snapshot subject identity.
        subject: String,
        /// Reason that capture cannot return a handle.
        reason: MissingDataReasonV1,
    },
}

impl CorpusCaptureV1 {
    fn key(&self) -> (&str, &str) {
        match self {
            Self::Available { snapshot } => (&snapshot.domain, &snapshot.subject),
            Self::Missing {
                domain, subject, ..
            } => (domain, subject),
        }
    }

    fn into_capture(self) -> SnapshotCaptureV1 {
        match self {
            Self::Available { snapshot } => SnapshotCaptureV1::Available {
                snapshot: Arc::new(snapshot),
            },
            Self::Missing { reason, .. } => SnapshotCaptureV1::Missing { reason },
        }
    }
}

/// One deterministic corpus input and its required result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CorpusCaseV1 {
    /// Stable case name.
    pub name: String,
    /// Host-attached request.
    pub request: SituationViewRequestV1,
    /// Domain capture states for the request.
    pub captures: Vec<CorpusCaptureV1>,
    /// Exact required result.
    pub expected: SituationViewResultV1,
}

/// Versioned collection of SituationView conformance cases.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SituationViewCorpusV1 {
    /// Corpus schema version.
    pub corpus_version: u16,
    /// Deterministic cases.
    pub cases: Vec<CorpusCaseV1>,
}

/// Loads and validates the linked V1 corpus.
///
/// # Errors
///
/// Returns [`SituationViewError::CorpusDecode`] for invalid JSON. Returns
/// [`SituationViewError::UnsupportedCorpusVersion`] for another version.
pub fn load_corpus_v1() -> Result<SituationViewCorpusV1, SituationViewError> {
    let corpus: SituationViewCorpusV1 = serde_json::from_str(CORPUS_JSON)
        .map_err(|source| SituationViewError::CorpusDecode { source })?;
    if corpus.corpus_version != SITUATION_VIEW_CORPUS_VERSION {
        return Err(SituationViewError::UnsupportedCorpusVersion {
            found: corpus.corpus_version,
            expected: SITUATION_VIEW_CORPUS_VERSION,
        });
    }
    Ok(corpus)
}

/// Runs the corpus through one implementation adapter.
///
/// The adapter receives the complete case. It can install the capture states
/// in its own test boundary before it evaluates the request.
///
/// # Errors
///
/// Returns the adapter error for a failed case. Returns
/// [`SituationViewError::CorpusMismatch`] when a result is not exact.
pub fn verify_corpus_v1<F>(
    corpus: &SituationViewCorpusV1,
    mut evaluate: F,
) -> Result<(), SituationViewError>
where
    F: FnMut(&CorpusCaseV1) -> Result<SituationViewResultV1, SituationViewError>,
{
    for case in &corpus.cases {
        let actual = evaluate(case)?;
        if actual != case.expected {
            return Err(SituationViewError::CorpusMismatch {
                case_name: case.name.clone(),
            });
        }
    }
    Ok(())
}

/// Runs the linked corpus through the reference composer.
///
/// # Errors
///
/// Returns a typed corpus, setup, request, or comparison error.
pub fn verify_reference_corpus_v1() -> Result<(), SituationViewError> {
    let corpus = load_corpus_v1()?;
    verify_corpus_v1(&corpus, evaluate_reference_case)
}

fn evaluate_reference_case(
    case: &CorpusCaseV1,
) -> Result<SituationViewResultV1, SituationViewError> {
    let source = CorpusSnapshotSourceV1::new(case)?;
    ComposingSituationViewV1::new(source).query(&case.request)
}

#[derive(Debug, Clone)]
struct CorpusSnapshotSourceV1 {
    captures: BTreeMap<(String, String), SnapshotCaptureV1>,
}

impl CorpusSnapshotSourceV1 {
    fn new(case: &CorpusCaseV1) -> Result<Self, SituationViewError> {
        let mut captures = BTreeMap::new();
        for capture in case.captures.iter().cloned() {
            let (domain, subject) = capture.key();
            let domain = domain.to_string();
            let subject = subject.to_string();
            let key = (domain.clone(), subject.clone());
            if captures.insert(key, capture.into_capture()).is_some() {
                return Err(SituationViewError::DuplicateCorpusCapture {
                    case_name: case.name.clone(),
                    domain,
                    subject,
                });
            }
        }
        Ok(Self { captures })
    }
}

impl SnapshotSourceV1 for CorpusSnapshotSourceV1 {
    fn capture(
        &self,
        selection: &crate::DomainSelectionV1,
        _time: &TimeQueryV1,
    ) -> SnapshotCaptureV1 {
        self.captures
            .get(&(selection.domain.clone(), selection.subject.clone()))
            .cloned()
            .unwrap_or(SnapshotCaptureV1::Missing {
                reason: MissingDataReasonV1::DomainUnavailable,
            })
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic)]
mod tests {
    use super::{load_corpus_v1, verify_reference_corpus_v1};

    #[test]
    fn linked_corpus_has_all_required_cases() {
        let corpus = load_corpus_v1().expect("corpus must decode");
        let names: Vec<&str> = corpus.cases.iter().map(|case| case.name.as_str()).collect();
        assert_eq!(
            names,
            vec![
                "mixed_age_inputs",
                "unknown_age",
                "missing_domain",
                "source_restart",
                "clock_correspondence_uncertainty",
            ]
        );
    }

    #[test]
    fn reference_composer_matches_linked_corpus() {
        verify_reference_corpus_v1().expect("reference composer must match corpus");
    }
}
