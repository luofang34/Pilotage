//! The scoped response limits the golden fixture stage decides against.

use flight_tune::Digest;

use super::scenario;

/// The scoped limit for every hidden decision the golden stage takes.
pub(super) fn response_targets(
    promotion_limit: f64,
    qualification_limit: f64,
) -> flight_tune::ResponseTargetTable {
    let mut rows = Vec::new();
    for (scenarios, limit) in [
        (vec![scenario("promotion-calm", 12)], promotion_limit),
        (
            vec![scenario("final-calm", 13), scenario("final-crosswind", 14)],
            qualification_limit,
        ),
    ] {
        for mission in scenarios {
            let mut envelope = *mission.content_digest.as_bytes();
            envelope[0] ^= 0xa5;
            rows.extend(
                flight_tune::ResponseTargetScope {
                    mission_revision_id: mission.revision_id.clone(),
                    mission_content_digest: mission.content_digest,
                    control_family: flight_tune::ControlFamily::OperatorVelocity,
                    control_channel: flight_tune::ControlChannel::Roll,
                    physical_target: flight_tune::PhysicalTarget {
                        unit: flight_tune::PhysicalUnit::MetersPerSecond,
                        value: 3.0,
                    },
                    envelope_digest: Digest::from_bytes(envelope),
                    authority_band: None,
                }
                .rows([("tracking", flight_tune::TargetComparison::AtMost, limit)]),
            );
        }
    }
    flight_tune::ResponseTargetTable::new(rows).expect("the golden response target table is valid")
}
