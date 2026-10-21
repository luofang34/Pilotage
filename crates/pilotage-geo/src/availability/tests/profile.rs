use super::*;

#[test]
fn the_same_reading_is_judged_by_the_selected_profile() {
    // A position 1-sigma that is fully fresh under the simulator profile but only
    // degraded under a stricter one: the verdict tracks the profile, not the
    // wire, so the same input yields different, deterministic results.
    let now = timestamp(SIM_FRESH_AGE_NS / 2);
    let mut pos = position(IntegrityLevel::Trusted, 0);
    pos.quality = PositionQuality {
        horizontal_mm: 1_000,
        vertical_mm: 1_000,
    };

    let stricter = AvailabilityProfile::new(
        AvailabilityProfileId(2),
        1,
        SIM_FRESH_AGE_NS,
        SIM_USABLE_AGE_NS,
        500,
        2_000,
        SIM_FRESH_ATT_MRAD,
        SIM_USABLE_ATT_MRAD,
    )
    .expect("a monotonic profile");

    let under_sim = derive_sim(
        &pos,
        &attitude(IntegrityLevel::Trusted, 0),
        &external_ok(),
        now,
    );
    let under_strict = derive_inputs(
        &pos,
        &attitude(IntegrityLevel::Trusted, 0),
        &external_ok(),
        now,
        &stricter,
    );

    assert_eq!(
        SvsAvailability::assess(&under_sim),
        SvsAvailability::Available,
    );
    assert_eq!(
        SvsAvailability::assess(&under_strict),
        SvsAvailability::Degraded(AvailabilityReason::Position),
    );
}

#[test]
fn the_age_boundary_is_pinned_under_the_simulator_profile() {
    let att = attitude(IntegrityLevel::Trusted, 0);
    let pos = position(IntegrityLevel::Trusted, 0);
    let assess = |reference: Timestamp| {
        SvsAvailability::assess(&derive_sim(&pos, &att, &external_ok(), reference))
    };
    // Exactly at the fresh age: still available.
    assert_eq!(
        assess(timestamp(SIM_FRESH_AGE_NS)),
        SvsAvailability::Available
    );
    // One nanosecond older: degraded on time/coherence.
    assert_eq!(
        assess(timestamp(SIM_FRESH_AGE_NS + 1)),
        SvsAvailability::Degraded(AvailabilityReason::TimeCoherence),
    );
    // Exactly at the usable age: still only degraded.
    assert_eq!(
        assess(timestamp(SIM_USABLE_AGE_NS)),
        SvsAvailability::Degraded(AvailabilityReason::TimeCoherence),
    );
    // One nanosecond beyond usable: unavailable.
    assert_eq!(
        assess(timestamp(SIM_USABLE_AGE_NS + 1)),
        SvsAvailability::Unavailable(AvailabilityReason::TimeCoherence),
    );
}
