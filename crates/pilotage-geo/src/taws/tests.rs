//! Tests pinning TAWS independence in the type surface.
#![allow(clippy::expect_used, clippy::panic)]

use super::TawsHazard;
use crate::availability::{AvailabilityReason, InputHealth, SvsAvailability, SvsInputs};

#[test]
fn taws_hazard_wire_codes_round_trip_and_reject_unknown() {
    for h in [TawsHazard::None, TawsHazard::Caution, TawsHazard::Warning] {
        assert_eq!(TawsHazard::from_u8(h.to_u8()), Some(h));
    }
    assert_eq!(TawsHazard::from_u8(9), None);
}

// A structural guarantee: the availability API takes only SvsInputs and yields
// only an SvsAvailability, with no TawsHazard anywhere in its signature — so a
// TAWS alert can neither influence the SVS verdict nor be produced by it.
#[test]
fn svs_availability_is_computed_without_any_taws_input() {
    let ok = InputHealth::Ok;
    let inputs = SvsInputs {
        position: ok,
        attitude: ok,
        integrity: ok,
        time_coherence: ok,
        calibration: ok,
        database: ok,
        coverage: ok,
        renderer: ok,
    };
    // An available scene says nothing about terrain; a caller must consult the
    // independent TAWS input separately.
    let verdict = SvsAvailability::assess(&inputs);
    assert_eq!(verdict, SvsAvailability::Available);
    assert_eq!(verdict.reason(), AvailabilityReason::Nominal);
}
