//! Group-level status reporting: the generic per-[`GroupId`] surface a
//! registry or admission harness asks, instead of a method per group.
//!
//! Each group's status is the group-level worst-of over the same inputs
//! its rendered signals fold — freshness, source trust, per-group
//! validation, declared validity. A group with several members (both
//! kinematic vectors, both dynamics samples) reports the worst member,
//! so this surface can only be more conservative than any one signal.

use crate::aircraft::AircraftState;
use crate::group_id::{GroupId, GroupStatuses};
use crate::signal::{FreshnessPolicy, SignalStatus};
use crate::validate::StateIntegrity;

use super::{Trust, fault_status, group_freshness};

/// The monitor channel's own slow policy: a live machine feed updates
/// irregularly and must not flap under the flight-data thresholds.
pub(crate) const TEXT_FRESHNESS: FreshnessPolicy =
    FreshnessPolicy::from_validated_literals(2000.0, 10_000.0);

pub(super) fn group_statuses(
    state: &AircraftState,
    policy: &FreshnessPolicy,
    trust: &Trust,
    integrity: &StateIntegrity,
) -> GroupStatuses {
    let mut out = GroupStatuses::default();
    for id in GroupId::ALL {
        out.set(id, group_status(state, policy, trust, integrity, id));
    }
    out
}

fn group_status(
    state: &AircraftState,
    policy: &FreshnessPolicy,
    trust: &Trust,
    integrity: &StateIntegrity,
    id: GroupId,
) -> SignalStatus {
    match id {
        GroupId::Attitude => {
            let has = state.attitude.data.is_some();
            let fresh = group_freshness(policy, has, state.attitude.age_ms);
            trust.fold(
                has,
                fresh,
                integrity.attitude.or(integrity.rates),
                state.valid.attitude && state.valid.rates,
            )
        }
        GroupId::Kinematics => {
            let has = state.kinematics.data.is_some();
            let fresh = group_freshness(policy, has, state.kinematics.age_ms);
            trust.fold(
                has,
                fresh,
                integrity.position.or(integrity.velocity),
                state.valid.position && state.valid.velocity,
            )
        }
        GroupId::Air => {
            let has = state.air.data.is_some();
            let fresh = group_freshness(policy, has, state.air.age_ms);
            trust.fold(has, fresh, integrity.air, true)
        }
        GroupId::Nav => {
            let has = state.nav.data.is_some();
            let fresh = group_freshness(policy, has, state.nav.age_ms);
            trust.fold(has, fresh, integrity.nav, true)
        }
        // Wind folds no source trust, mirroring its resolved signal: a
        // wind estimate is advisory and independently stamped.
        GroupId::Wind => group_freshness(policy, state.wind.data.is_some(), state.wind.age_ms)
            .worst(fault_status(integrity.wind)),
        GroupId::Selections => fault_status(integrity.selections),
        // Absent trust is fail-closed Failed, never Missing: trust must
        // be declared before any estimate group can show Valid.
        GroupId::Trust => trust.quality.worst(trust.coherence),
        GroupId::Altitude => fault_status(integrity.altitude),
        GroupId::Heading => {
            let has = state.heading.data.is_some();
            let fresh = group_freshness(policy, has, state.heading.age_ms);
            trust.fold(has, fresh, integrity.heading, state.valid.heading)
        }
        GroupId::Variation => {
            let has = state.variation.data.is_some();
            let fresh = group_freshness(policy, has, state.variation.age_ms);
            trust.fold(has, fresh, integrity.variation, state.valid.variation)
        }
        GroupId::Dynamics => {
            let has = state.dynamics.data.is_some();
            let fresh = group_freshness(policy, has, state.dynamics.age_ms);
            trust.fold(
                has,
                fresh,
                integrity.dynamics,
                state.valid.turn && state.valid.slip,
            )
        }
        // Advisory machine text folds no flight-source trust and runs
        // its own slow freshness policy, mirroring wind's independence.
        GroupId::MonitorText => group_freshness(
            &TEXT_FRESHNESS,
            state.monitor_text.data.is_some(),
            state.monitor_text.age_ms,
        )
        .worst(fault_status(integrity.monitor_text)),
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic)]
mod tests {
    use super::TEXT_FRESHNESS;
    use crate::signal::FreshnessPolicy;

    #[test]
    fn text_freshness_passes_the_validating_constructor() {
        let validated = FreshnessPolicy::new(
            TEXT_FRESHNESS.stale_after_ms(),
            TEXT_FRESHNESS.fail_after_ms(),
        )
        .expect("literal thresholds validate");
        assert_eq!(validated, TEXT_FRESHNESS);
    }
}
