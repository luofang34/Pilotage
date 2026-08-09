//! Age calculation with explicit clock evidence.

use crate::{
    AgeUnknownReasonV1, AgeV1, ClockCorrespondenceV1, EvidenceV1, MonotonicStampV1, TimeQualityV1,
    UtcInstantV1,
};

pub(super) fn ingress_age(
    ingress: &EvidenceV1<MonotonicStampV1>,
    evaluation: &MonotonicStampV1,
    correspondences: &[ClockCorrespondenceV1],
) -> AgeV1 {
    let EvidenceV1::Present { value: ingress } = ingress else {
        return AgeV1::Unknown {
            reason: AgeUnknownReasonV1::MissingIngressTime,
        };
    };
    let mapped = map_ingress(ingress, evaluation, correspondences);
    let Ok((mapped_nanoseconds, uncertainty_nanoseconds)) = mapped else {
        return AgeV1::Unknown {
            reason: mapped
                .err()
                .unwrap_or(AgeUnknownReasonV1::InvalidClockCorrespondence),
        };
    };
    let Some(nanoseconds) = evaluation.nanoseconds.checked_sub(mapped_nanoseconds) else {
        return AgeV1::Unknown {
            reason: AgeUnknownReasonV1::IngressAfterEvaluation,
        };
    };
    AgeV1::Known {
        nanoseconds,
        uncertainty_nanoseconds: EvidenceV1::Present {
            value: uncertainty_nanoseconds,
        },
    }
}

fn map_ingress(
    ingress: &MonotonicStampV1,
    evaluation: &MonotonicStampV1,
    correspondences: &[ClockCorrespondenceV1],
) -> Result<(u64, u64), AgeUnknownReasonV1> {
    if ingress.clock_id == evaluation.clock_id {
        return Ok((ingress.nanoseconds, 0));
    }
    let candidates: Vec<&ClockCorrespondenceV1> = correspondences
        .iter()
        .filter(|item| {
            item.source_clock_id == ingress.clock_id && item.target_clock_id == evaluation.clock_id
        })
        .collect();
    if candidates.is_empty() {
        return Err(AgeUnknownReasonV1::MissingClockCorrespondence);
    }
    if candidates
        .iter()
        .any(|item| item.valid_source.start_nanoseconds > item.valid_source.end_nanoseconds)
    {
        return Err(AgeUnknownReasonV1::InvalidClockCorrespondence);
    }
    let valid: Vec<&ClockCorrespondenceV1> = candidates
        .into_iter()
        .filter(|item| item.valid_source.contains(ingress.nanoseconds))
        .collect();
    match valid.as_slice() {
        [] => Err(AgeUnknownReasonV1::ClockCorrespondenceOutOfRange),
        [correspondence] => translate(ingress.nanoseconds, correspondence),
        _ => Err(AgeUnknownReasonV1::AmbiguousClockCorrespondence),
    }
}

fn translate(
    source_nanoseconds: u64,
    correspondence: &ClockCorrespondenceV1,
) -> Result<(u64, u64), AgeUnknownReasonV1> {
    if correspondence.valid_source.start_nanoseconds > correspondence.valid_source.end_nanoseconds {
        return Err(AgeUnknownReasonV1::InvalidClockCorrespondence);
    }
    let mapped = i128::from(source_nanoseconds)
        .checked_add(i128::from(correspondence.offset_nanoseconds))
        .ok_or(AgeUnknownReasonV1::InvalidClockCorrespondence)?;
    let mapped =
        u64::try_from(mapped).map_err(|_| AgeUnknownReasonV1::InvalidClockCorrespondence)?;
    Ok((mapped, correspondence.uncertainty_nanoseconds))
}

pub(super) fn observation_age(
    source_time: &EvidenceV1<UtcInstantV1>,
    time_quality: &EvidenceV1<TimeQualityV1>,
    uncertainty: &EvidenceV1<u64>,
    evaluation: UtcInstantV1,
) -> AgeV1 {
    let EvidenceV1::Present { value: source_time } = source_time else {
        return unknown(AgeUnknownReasonV1::MissingSourceTime);
    };
    let EvidenceV1::Present {
        value: time_quality,
    } = time_quality
    else {
        return unknown(AgeUnknownReasonV1::MissingTimeQuality);
    };
    if matches!(time_quality, TimeQualityV1::Untrusted) {
        return unknown(AgeUnknownReasonV1::UntrustedSourceTime);
    }
    let Some(evaluation_ns) = evaluation.unix_nanoseconds() else {
        return unknown(AgeUnknownReasonV1::InvalidUtcTime);
    };
    let Some(source_ns) = source_time.unix_nanoseconds() else {
        return unknown(AgeUnknownReasonV1::InvalidUtcTime);
    };
    let Some(age_ns) = evaluation_ns.checked_sub(source_ns) else {
        return unknown(AgeUnknownReasonV1::AgeOverflow);
    };
    if age_ns < 0 {
        return unknown(AgeUnknownReasonV1::SourceTimeAfterEvaluation);
    }
    let Ok(nanoseconds) = u64::try_from(age_ns) else {
        return unknown(AgeUnknownReasonV1::AgeOverflow);
    };
    AgeV1::Known {
        nanoseconds,
        uncertainty_nanoseconds: uncertainty.clone(),
    }
}

const fn unknown(reason: AgeUnknownReasonV1) -> AgeV1 {
    AgeV1::Unknown { reason }
}
