//! Reference composition over immutable snapshot handles.

use std::collections::BTreeSet;

use crate::{
    AvailableDomainResultV1, ConsistencyGuaranteeV1, ContributorResultV1, DomainResultV1,
    DomainSelectionV1, FieldResultV1, SITUATION_VIEW_SCHEMA_VERSION, SituationViewError,
    SituationViewRequestV1, SituationViewResultV1, SnapshotCaptureV1, SnapshotSourceV1,
};

mod age;
mod requirements;

/// Version 1 read-only situation query interface.
pub trait SituationViewV1 {
    /// Error returned by the implementation.
    type Error;

    /// Evaluates one host-attached request.
    fn query(&self, request: &SituationViewRequestV1)
    -> Result<SituationViewResultV1, Self::Error>;
}

/// Reference composer that captures from one source.
#[derive(Debug, Clone)]
pub struct ComposingSituationViewV1<S> {
    source: S,
}

impl<S> ComposingSituationViewV1<S> {
    /// Creates a composer for `source`.
    #[must_use]
    pub const fn new(source: S) -> Self {
        Self { source }
    }

    /// Returns the snapshot source.
    #[must_use]
    pub const fn source(&self) -> &S {
        &self.source
    }
}

impl<S: SnapshotSourceV1> SituationViewV1 for ComposingSituationViewV1<S> {
    type Error = SituationViewError;

    fn query(
        &self,
        request: &SituationViewRequestV1,
    ) -> Result<SituationViewResultV1, Self::Error> {
        validate_request(request)?;
        let domains = capture_domains(&self.source, request)?;
        let requirement_assessments = requirements::assess(request, &domains);
        Ok(SituationViewResultV1 {
            schema_version: SITUATION_VIEW_SCHEMA_VERSION,
            query_time: request.query.time.clone(),
            host_evaluation: request.host_evaluation.clone(),
            consistency: ConsistencyGuaranteeV1::BestAvailableNonAtomic,
            domains,
            requirement_assessments,
        })
    }
}

fn validate_request(request: &SituationViewRequestV1) -> Result<(), SituationViewError> {
    if request.schema_version != SITUATION_VIEW_SCHEMA_VERSION {
        return Err(SituationViewError::UnsupportedSchemaVersion {
            found: request.schema_version,
            expected: SITUATION_VIEW_SCHEMA_VERSION,
        });
    }
    validate_utc(request)?;
    validate_domain_selections(&request.query.domains)?;
    validate_requirement_ids(request)
}

fn validate_utc(request: &SituationViewRequestV1) -> Result<(), SituationViewError> {
    let instant = request.query.time.evaluation_utc;
    if instant.unix_nanoseconds().is_some() {
        Ok(())
    } else {
        Err(SituationViewError::InvalidUtcInstant {
            location: "request.query.time.evaluation_utc".to_string(),
            subsecond_nanoseconds: instant.subsecond_nanoseconds,
        })
    }
}

fn validate_domain_selections(selections: &[DomainSelectionV1]) -> Result<(), SituationViewError> {
    let mut seen = BTreeSet::new();
    for selection in selections {
        let key = (selection.domain.clone(), selection.subject.clone());
        if !seen.insert(key) {
            return Err(SituationViewError::DuplicateDomainSelection {
                domain: selection.domain.clone(),
                subject: selection.subject.clone(),
            });
        }
    }
    Ok(())
}

fn validate_requirement_ids(request: &SituationViewRequestV1) -> Result<(), SituationViewError> {
    let freshness = request
        .query
        .requirements
        .freshness
        .iter()
        .map(|item| item.requirement_id.as_str());
    let coherence = request
        .query
        .requirements
        .coherence
        .iter()
        .map(|item| item.requirement_id.as_str());
    let mut seen = BTreeSet::new();
    for requirement_id in freshness.chain(coherence) {
        if !seen.insert(requirement_id) {
            return Err(SituationViewError::DuplicateRequirementId {
                requirement_id: requirement_id.to_string(),
            });
        }
    }
    Ok(())
}

fn capture_domains<S: SnapshotSourceV1>(
    source: &S,
    request: &SituationViewRequestV1,
) -> Result<Vec<DomainResultV1>, SituationViewError> {
    request
        .query
        .domains
        .iter()
        .map(|selection| {
            let capture = source.capture(selection, &request.query.time);
            compose_capture(selection, capture, request)
        })
        .collect()
}

fn compose_capture(
    selection: &DomainSelectionV1,
    capture: SnapshotCaptureV1,
    request: &SituationViewRequestV1,
) -> Result<DomainResultV1, SituationViewError> {
    match capture {
        SnapshotCaptureV1::Available { snapshot } => {
            validate_capture(selection, &snapshot)?;
            let fields = snapshot
                .fields
                .iter()
                .map(|field| compose_field(field, &snapshot.clock_correspondences, request))
                .collect();
            Ok(DomainResultV1::Available {
                domain: selection.domain.clone(),
                subject: selection.subject.clone(),
                result: AvailableDomainResultV1 {
                    domain_schema_version: snapshot.domain_schema_version,
                    producer_instance_id: snapshot.producer_instance_id.clone(),
                    snapshot_revision: snapshot.snapshot_revision,
                    domain_identity: snapshot.domain_identity.clone(),
                    fields,
                    clock_correspondences: snapshot.clock_correspondences.clone(),
                },
            })
        }
        SnapshotCaptureV1::Missing { reason } => Ok(DomainResultV1::Missing {
            domain: selection.domain.clone(),
            subject: selection.subject.clone(),
            reason,
        }),
    }
}

fn validate_capture(
    selection: &DomainSelectionV1,
    snapshot: &crate::DomainSnapshotV1,
) -> Result<(), SituationViewError> {
    if snapshot.domain == selection.domain && snapshot.subject == selection.subject {
        return Ok(());
    }
    Err(SituationViewError::SnapshotSelectionMismatch {
        selected_domain: selection.domain.clone(),
        selected_subject: selection.subject.clone(),
        actual_domain: snapshot.domain.clone(),
        actual_subject: snapshot.subject.clone(),
    })
}

fn compose_field(
    field: &crate::FieldSnapshotV1,
    correspondences: &[crate::ClockCorrespondenceV1],
    request: &SituationViewRequestV1,
) -> FieldResultV1 {
    let contributors = field
        .contributors
        .iter()
        .map(|evidence| ContributorResultV1 {
            evidence: evidence.clone(),
            ingress_age: age::ingress_age(
                &evidence.ingress_time,
                &request.host_evaluation,
                correspondences,
            ),
            observation_age: age::observation_age(
                &evidence.source_time,
                &evidence.time_quality,
                &evidence.time_uncertainty_nanoseconds,
                request.query.time.evaluation_utc,
            ),
        })
        .collect();
    FieldResultV1 {
        field: field.field.clone(),
        value: field.value.clone(),
        contributors,
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic)]
mod tests;
