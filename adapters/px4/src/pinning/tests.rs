use std::time::{Duration, Instant};

use pilotage_mavlink::codec::{PARAM_SET_ID, ParamValue, param_id};

use super::{ParamPinning, ParamRefusal};
use crate::config::Px4Profile;

/// PARAM_REQUEST_READ's message id.
const PARAM_REQUEST_READ_ID: u32 = 20;
const TABLE: [(&str, ParamValue); 2] = [
    ("COM_OBL_RC_ACT", ParamValue::Int32(5)),
    ("FW_R_LIM", ParamValue::Real32(35.0)),
];

fn message_ids(plan: &mut ParamPinning, now: Instant) -> Vec<u32> {
    plan.due(now)
        .into_iter()
        .map(|request| {
            let frame = request.encode(0, 1, 1);
            u32::from(frame[7]) | (u32::from(frame[8]) << 8) | (u32::from(frame[9]) << 16)
        })
        .collect()
}

#[test]
fn only_the_simulation_profile_writes() {
    let now = Instant::now();
    let mut simulation = ParamPinning::new(Px4Profile::Simulation, &TABLE);
    assert_eq!(message_ids(&mut simulation, now), [PARAM_SET_ID; 2]);
    let mut physical = ParamPinning::new(Px4Profile::Physical, &TABLE);
    assert_eq!(message_ids(&mut physical, now), [PARAM_REQUEST_READ_ID; 2]);
}

#[test]
fn a_physical_mismatch_is_reported_and_read_again_never_written() {
    let start = Instant::now();
    let mut physical = ParamPinning::new(Px4Profile::Physical, &TABLE);
    message_ids(&mut physical, start);
    physical.observe(param_id("COM_OBL_RC_ACT"), ParamValue::Int32(0));
    physical.observe(param_id("FW_R_LIM"), ParamValue::Real32(35.0));
    assert_eq!(
        physical.refusal(),
        Some(ParamRefusal::Mismatch {
            name: "COM_OBL_RC_ACT",
            expected: ParamValue::Int32(5),
            found: ParamValue::Int32(0),
        })
    );
    assert!(!physical.all_confirmed());
    let retry = start + Duration::from_secs(1);
    assert_eq!(message_ids(&mut physical, retry), [PARAM_REQUEST_READ_ID]);
    physical.observe(param_id("COM_OBL_RC_ACT"), ParamValue::Int32(5));
    assert_eq!(physical.refusal(), None);
}

#[test]
fn a_simulation_mismatch_waits_for_the_write_to_land() {
    let start = Instant::now();
    let mut simulation = ParamPinning::new(Px4Profile::Simulation, &TABLE);
    message_ids(&mut simulation, start);
    simulation.observe(param_id("COM_OBL_RC_ACT"), ParamValue::Int32(0));
    assert_eq!(
        simulation.refusal(),
        Some(ParamRefusal::Unconfirmed {
            name: "COM_OBL_RC_ACT",
            expected: ParamValue::Int32(5),
        })
    );
    let retry = start + Duration::from_secs(1);
    assert_eq!(message_ids(&mut simulation, retry), [PARAM_SET_ID; 2]);
}
