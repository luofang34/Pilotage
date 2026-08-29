//! The scoped response limits the rig stage decides against.

use flight_tune::MissionReference;

/// The scoped limit for every hidden decision the rig stage takes.
///
/// The rig commands one operator velocity roll axis, so every scope names the
/// same family and unit and differs only by the scenario it measures.
pub fn response_targets(
    promotion: &[MissionReference],
    final_qualification: &[MissionReference],
) -> flight_tune::ResponseTargetTable {
    let mut rows = Vec::new();
    for (scenarios, limit) in [(promotion, 1.0), (final_qualification, 0.75)] {
        for scenario in scenarios {
            rows.extend(rig_scope(scenario).rows([(
                "test.response",
                flight_tune::TargetComparison::AtMost,
                limit,
            )]));
        }
    }
    flight_tune::ResponseTargetTable::new(rows).expect("the rig response target table is valid")
}

fn rig_scope(scenario: &MissionReference) -> flight_tune::ResponseTargetScope {
    flight_tune::ResponseTargetScope {
        mission_revision_id: scenario.revision_id.clone(),
        mission_content_digest: scenario.content_digest,
        control_family: flight_tune::ControlFamily::OperatorVelocity,
        control_channel: flight_tune::ControlChannel::Roll,
        physical_target: flight_tune::PhysicalTarget {
            unit: flight_tune::PhysicalUnit::MetersPerSecond,
            value: 3.0,
        },
        envelope_digest: super::super::fake_operator_envelope_digest(),
        authority_band: None,
    }
}
