//! Freshness and coherence assessment.

use serde_json::Value;

use crate::{
    AgeV1, CoherenceRequirementV1, CoherenceRuleV1, DomainResultV1, EvidenceV1, FieldResultV1,
    FieldScopeV1, FreshnessAgeV1, FreshnessRequirementV1, RequirementAssessmentV1,
    RequirementReasonV1, RequirementStatusV1, SituationViewRequestV1, SnapshotRequirementV1,
};

pub(super) fn assess(
    request: &SituationViewRequestV1,
    domains: &[DomainResultV1],
) -> Vec<RequirementAssessmentV1> {
    let freshness = request
        .query
        .requirements
        .freshness
        .iter()
        .map(|item| assess_freshness(item, domains));
    let coherence = request
        .query
        .requirements
        .coherence
        .iter()
        .map(|item| assess_coherence(item, domains));
    freshness.chain(coherence).collect()
}

fn assess_freshness(
    requirement: &FreshnessRequirementV1,
    domains: &[DomainResultV1],
) -> RequirementAssessmentV1 {
    let status = match find_field(domains, &requirement.field) {
        FieldLookup::MissingDomain => not_satisfied(RequirementReasonV1::MissingDomain),
        FieldLookup::MissingField => not_satisfied(RequirementReasonV1::MissingField),
        FieldLookup::Present(field) => freshness_status(requirement, field),
    };
    RequirementAssessmentV1 {
        requirement_id: requirement.requirement_id.clone(),
        status,
    }
}

fn freshness_status(
    requirement: &FreshnessRequirementV1,
    field: &FieldResultV1,
) -> RequirementStatusV1 {
    if matches!(field.value, EvidenceV1::Missing { .. }) {
        return not_satisfied(RequirementReasonV1::MissingField);
    }
    if field.contributors.is_empty() {
        return indeterminate(RequirementReasonV1::MissingContributor);
    }
    for contributor in &field.contributors {
        let age = match requirement.age {
            FreshnessAgeV1::Ingress => &contributor.ingress_age,
            FreshnessAgeV1::Observation => &contributor.observation_age,
        };
        match age_upper_bound(age) {
            AgeBound::Known(upper) if upper > requirement.maximum_age_nanoseconds => {
                return not_satisfied(RequirementReasonV1::MaximumAgeExceeded);
            }
            AgeBound::Known(_) => {}
            AgeBound::UnknownAge => {
                return indeterminate(RequirementReasonV1::UnknownAge);
            }
            AgeBound::UnknownUncertainty => {
                return indeterminate(RequirementReasonV1::UnknownUncertainty);
            }
        }
    }
    RequirementStatusV1::Satisfied
}

fn assess_coherence(
    requirement: &CoherenceRequirementV1,
    domains: &[DomainResultV1],
) -> RequirementAssessmentV1 {
    let status = match &requirement.rule {
        CoherenceRuleV1::MaximumIngressAgeSpread {
            fields,
            maximum_spread_nanoseconds,
        } => spread_status(fields, *maximum_spread_nanoseconds, domains),
        CoherenceRuleV1::EqualFieldValues { fields } => equal_value_status(fields, domains),
        CoherenceRuleV1::ExactSnapshots { snapshots } => exact_snapshot_status(snapshots, domains),
    };
    RequirementAssessmentV1 {
        requirement_id: requirement.requirement_id.clone(),
        status,
    }
}

fn spread_status(
    scopes: &[FieldScopeV1],
    maximum: u64,
    domains: &[DomainResultV1],
) -> RequirementStatusV1 {
    let mut lower = u64::MAX;
    let mut upper = 0_u64;
    let mut count = 0_usize;
    for scope in scopes {
        let FieldLookup::Present(field) = find_field(domains, scope) else {
            return not_satisfied(missing_scope_reason(domains, scope));
        };
        if field.contributors.is_empty() {
            return indeterminate(RequirementReasonV1::MissingContributor);
        }
        for contributor in &field.contributors {
            let Some((item_lower, item_upper)) = age_interval(&contributor.ingress_age) else {
                return indeterminate(age_reason(&contributor.ingress_age));
            };
            lower = lower.min(item_lower);
            upper = upper.max(item_upper);
            count = count.saturating_add(1);
        }
    }
    if count == 0 {
        return indeterminate(RequirementReasonV1::MissingContributor);
    }
    if upper.saturating_sub(lower) <= maximum {
        RequirementStatusV1::Satisfied
    } else {
        not_satisfied(RequirementReasonV1::MaximumSpreadExceeded)
    }
}

fn equal_value_status(scopes: &[FieldScopeV1], domains: &[DomainResultV1]) -> RequirementStatusV1 {
    let mut expected: Option<&Value> = None;
    for scope in scopes {
        let lookup = find_field(domains, scope);
        let FieldLookup::Present(field) = lookup else {
            return not_satisfied(missing_scope_reason(domains, scope));
        };
        let EvidenceV1::Present { value } = &field.value else {
            return not_satisfied(RequirementReasonV1::MissingField);
        };
        if let Some(first) = expected
            && first != value
        {
            return not_satisfied(RequirementReasonV1::FieldValuesDiffer);
        }
        expected = Some(value);
    }
    RequirementStatusV1::Satisfied
}

fn exact_snapshot_status(
    requirements: &[SnapshotRequirementV1],
    domains: &[DomainResultV1],
) -> RequirementStatusV1 {
    for requirement in requirements {
        let found = domains.iter().find(|domain| match domain {
            DomainResultV1::Available {
                domain, subject, ..
            }
            | DomainResultV1::Missing {
                domain, subject, ..
            } => domain == &requirement.domain && subject == &requirement.subject,
        });
        match found {
            Some(DomainResultV1::Available { result, .. })
                if result.producer_instance_id == requirement.producer_instance_id
                    && result.snapshot_revision == requirement.snapshot_revision => {}
            Some(DomainResultV1::Missing { .. }) | None => {
                return not_satisfied(RequirementReasonV1::MissingDomain);
            }
            Some(DomainResultV1::Available { .. }) => {
                return not_satisfied(RequirementReasonV1::SnapshotIdentityMismatch);
            }
        }
    }
    RequirementStatusV1::Satisfied
}

enum FieldLookup<'a> {
    MissingDomain,
    MissingField,
    Present(&'a FieldResultV1),
}

fn find_field<'a>(domains: &'a [DomainResultV1], scope: &FieldScopeV1) -> FieldLookup<'a> {
    let domain = domains.iter().find(|domain| match domain {
        DomainResultV1::Available {
            domain, subject, ..
        }
        | DomainResultV1::Missing {
            domain, subject, ..
        } => domain == &scope.domain && subject == &scope.subject,
    });
    let Some(DomainResultV1::Available { result, .. }) = domain else {
        return FieldLookup::MissingDomain;
    };
    result
        .fields
        .iter()
        .find(|field| field.field == scope.field)
        .map_or(FieldLookup::MissingField, FieldLookup::Present)
}

enum AgeBound {
    Known(u64),
    UnknownAge,
    UnknownUncertainty,
}

fn age_upper_bound(age: &AgeV1) -> AgeBound {
    match age {
        AgeV1::Known {
            nanoseconds,
            uncertainty_nanoseconds: EvidenceV1::Present { value },
        } => AgeBound::Known(nanoseconds.saturating_add(*value)),
        AgeV1::Known { .. } => AgeBound::UnknownUncertainty,
        AgeV1::Unknown { .. } => AgeBound::UnknownAge,
    }
}

fn age_interval(age: &AgeV1) -> Option<(u64, u64)> {
    let AgeV1::Known {
        nanoseconds,
        uncertainty_nanoseconds: EvidenceV1::Present { value },
    } = age
    else {
        return None;
    };
    Some((
        nanoseconds.saturating_sub(*value),
        nanoseconds.saturating_add(*value),
    ))
}

fn age_reason(age: &AgeV1) -> RequirementReasonV1 {
    match age {
        AgeV1::Known { .. } => RequirementReasonV1::UnknownUncertainty,
        AgeV1::Unknown { .. } => RequirementReasonV1::UnknownAge,
    }
}

fn missing_scope_reason(domains: &[DomainResultV1], scope: &FieldScopeV1) -> RequirementReasonV1 {
    match find_field(domains, scope) {
        FieldLookup::MissingDomain => RequirementReasonV1::MissingDomain,
        FieldLookup::MissingField | FieldLookup::Present(_) => RequirementReasonV1::MissingField,
    }
}

const fn not_satisfied(reason: RequirementReasonV1) -> RequirementStatusV1 {
    RequirementStatusV1::NotSatisfied { reason }
}

const fn indeterminate(reason: RequirementReasonV1) -> RequirementStatusV1 {
    RequirementStatusV1::Indeterminate { reason }
}
