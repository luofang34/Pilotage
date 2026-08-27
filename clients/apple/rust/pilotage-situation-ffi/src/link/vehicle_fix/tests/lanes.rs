//! Which lane the mark is read from, and whether it may be read at all.

#![allow(clippy::expect_used, clippy::panic)]

use super::*;

#[test]
fn a_sample_with_no_lane_states_no_position() {
    // Absence is not the origin. A map that draws 0,0 for "nothing known"
    // puts the vehicle in the Gulf of Guinea.
    assert!(first_sample(&wire::TelemetrySample::default()).is_none());
}

#[test]
fn truth_without_its_stamp_is_unconsumable() {
    // A position that cannot be shown to be this vehicle's, this boot's and
    // this moment's is not drawn, however plausible its numbers.
    let mut sample = truth(3.0, 0.0);
    sample.sim_truth.as_mut().expect("truth lane").stamp = None;
    assert!(first_sample(&sample).is_none());
}

#[test]
fn the_oracle_is_preferred_and_says_so() {
    let read = first_sample(&truth(3.0, 0.0)).expect("a fix");
    assert!(read.from_simulator);
    assert!((read.latitude_deg - 47.397_742).abs() < 1e-9);
}

#[test]
fn the_estimate_lane_needs_its_status_observation() {
    // The mask on the estimate lane is meaningless without the observation
    // backing it, so its directions are withheld rather than assumed good.
    let sample = wire::TelemetrySample {
        avionics: Some(wire::AvionicsState {
            quat_w: 1.0,
            vel_n_mps: 3.0,
            valid_flags: ATTITUDE_AND_VELOCITY,
            geodetic: Some(fix()),
            geodetic_stamp: Some(estimate_fix_stamp(1)),
            attitude_stamp: Some(stamp()),
            kinematics_stamp: Some(stamp()),
            ..Default::default()
        }),
        ..Default::default()
    };
    let read = first_sample(&sample).expect("a fix");
    assert!(!read.from_simulator);
    assert_eq!(
        read.heading_deg, None,
        "no status observation authorizes it"
    );
    assert_eq!(read.course_deg, None);

    // With the observation present the same sample states both.
    let mut authorized = sample;
    authorized
        .avionics
        .as_mut()
        .expect("estimate lane")
        .estimator_status_stamp = Some(stamp());
    let read = first_sample(&authorized).expect("a fix");
    assert_eq!(read.heading_deg, Some(0.0));
    assert_eq!(read.course_deg, Some(0.0));
}

#[test]
fn the_fix_is_withheld_when_the_estimate_states_no_position() {
    // The geodetic fix carries its own stamp and advances on its own, so a
    // lane with attitude but no position draws no mark at all.
    let sample = wire::TelemetrySample {
        avionics: Some(wire::AvionicsState {
            quat_w: 1.0,
            valid_flags: ATTITUDE_AND_VELOCITY,
            estimator_status_stamp: Some(stamp()),
            ..Default::default()
        }),
        ..Default::default()
    };
    assert!(first_sample(&sample).is_none());
}

#[test]
fn an_estimator_that_calls_its_own_solution_unusable_is_believed() {
    // The clearest refusal there is. Reading the directions anyway would turn
    // the mark by a number the estimator itself disowned — and the browser
    // withholds them, so drawing them here would put the two clients on
    // different answers from one sample.
    let sample = wire::TelemetrySample {
        avionics: Some(wire::AvionicsState {
            quat_w: 1.0,
            vel_n_mps: 3.0,
            valid_flags: ATTITUDE_AND_VELOCITY,
            estimator_status_stamp: Some(stamp()),
            quality: 2,
            geodetic: Some(fix()),
            geodetic_stamp: Some(estimate_fix_stamp(1)),
            attitude_stamp: Some(stamp()),
            kinematics_stamp: Some(stamp()),
            ..Default::default()
        }),
        ..Default::default()
    };
    let read = first_sample(&sample).expect("the position still stands");
    assert_eq!(
        read.heading_deg, None,
        "an unusable solution turned the mark"
    );
    assert_eq!(read.course_deg, None, "an unusable solution drew a course");
    assert_eq!(read.ground_speed_mps, None);

    // Degraded is not unusable: the estimator is still standing behind it.
    let mut degraded = sample;
    degraded.avionics.as_mut().expect("estimate lane").quality = 1;
    let read = first_sample(&degraded).expect("a fix");
    assert_eq!(
        read.heading_deg,
        Some(0.0),
        "a degraded solution was refused"
    );
}

#[test]
fn a_lane_stamped_with_another_lanes_role_is_not_that_lane() {
    // Role travels in provenance so a group cannot be read as something it is
    // not. The browser settles this in its decoder and a mislabelled group
    // never reaches the map; this client decodes without that step, so the
    // refusal has to be made here or not at all.
    let mut sample = truth(3.0, 0.0);
    sample
        .sim_truth
        .as_mut()
        .expect("truth lane")
        .stamp
        .as_mut()
        .expect("a stamp")
        .role = 1;
    assert!(
        first_sample(&sample).is_none(),
        "an estimate-stamped block was drawn as the simulator's oracle"
    );
}

#[test]
fn a_truth_stamped_position_is_not_read_as_the_estimators_answer() {
    // The substitution the roles exist to stop: an oracle's position, exact by
    // construction, presented as a solution the estimator stands behind.
    let mut sample = estimate(1, 1, 1);
    sample
        .avionics
        .as_mut()
        .expect("estimate lane")
        .geodetic_stamp
        .as_mut()
        .expect("a stamp")
        .role = 2;
    assert!(
        first_sample(&sample).is_none(),
        "a truth-stamped fix was drawn as the estimator's own"
    );
}

#[test]
fn a_position_stating_no_datum_is_not_placed() {
    // Two datums put the same degrees a couple of metres apart, and which one
    // was meant is not recoverable afterwards. The schema says unknown is
    // refused at the receiver and never guessed.
    assert!(
        with_position(|fix| fix.horizontal_datum = 0).is_none(),
        "a position with no datum was placed on the map"
    );
    assert!(
        with_position(|fix| fix.horizontal_datum = 9).is_none(),
        "a position was placed against a datum this build does not know"
    );
    // A datum that is a realization of a frame needs to say which.
    assert!(
        with_position(|fix| {
            fix.horizontal_datum = 2;
            fix.realization = 0;
        })
        .is_none(),
        "a realization-bearing datum was accepted without one"
    );
    assert!(
        with_position(|fix| {
            fix.horizontal_datum = 2;
            fix.realization = 1;
        })
        .is_some(),
        "a datum stating its realization was refused"
    );
}

#[test]
fn a_position_that_is_not_on_the_earth_is_not_drawn() {
    // The wire says the producer sends a normalized longitude. One that needs
    // wrapping is a producer that did not keep the contract, and wrapping it
    // here would draw the vehicle a full turn of the Earth from where the
    // other client draws nothing at all.
    for (name, edit) in [
        ("a latitude past the pole", 91.0_f64, 0.0_f64),
        ("a longitude past the antimeridian", 0.0, 180.0),
        ("a longitude below the antimeridian", 0.0, -180.001),
    ]
    .map(|(name, lat, lon)| {
        (name, move |fix: &mut wire::GeodeticFix| {
            fix.latitude_deg = lat;
            fix.longitude_deg = lon;
        })
    }) {
        assert!(with_position(edit).is_none(), "{name} was drawn");
    }
    assert!(
        with_position(|fix| fix.latitude_deg = f64::NAN).is_none(),
        "a position that is not a number was drawn"
    );
    // The antimeridian itself is a place: the range is half-open, so -180 is
    // on the map and +180 is the same meridian stated the way the wire does
    // not allow.
    assert!(
        with_position(|fix| fix.longitude_deg = -180.0).is_some(),
        "the antimeridian was refused"
    );
}
