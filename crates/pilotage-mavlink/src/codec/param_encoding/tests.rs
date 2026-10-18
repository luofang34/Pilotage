use super::{ParamEncoding, ParamValue};

#[test]
fn one_frame_reads_as_each_autopilot_means_it() {
    // GUID_OPTIONS = 8 from ArduPilot: the float 8.0, typed INT32.
    let ardupilot = ParamValue::Int32(8.0_f32.to_bits() as i32);
    assert_eq!(ParamEncoding::CCast.number(ardupilot), 8.0);
    assert_ne!(ParamEncoding::Bytewise.number(ardupilot), 8.0);
    assert_eq!(ParamEncoding::Bytewise.number(ParamValue::Int32(8)), 8.0);
    assert_eq!(
        ParamEncoding::CCast.number(ParamValue::Real32(0.39)),
        f64::from(0.39_f32)
    );
}

#[test]
fn an_integer_write_round_trips_under_its_own_encoding() {
    for encoding in [ParamEncoding::Bytewise, ParamEncoding::CCast] {
        assert_eq!(encoding.number(encoding.integer(9)), 9.0);
    }
    assert_eq!(ParamEncoding::Bytewise.integer(9), ParamValue::Int32(9));
}
