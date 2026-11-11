//! The stated magnetic variation on the wire: what it carries, and what
//! the mapping refuses to send.

use pilotage_adapter_api::{
    AvionicsSample, MagneticVariationSample, MagneticVariationSource, MeasurementClock,
    MeasurementStamp, SourceIncarnation, SourceIntegrity, SourceRole,
};
use pilotage_protocol::wire;

fn estimate_stamp() -> MeasurementStamp {
    MeasurementStamp {
        role: SourceRole::OperationalEstimate,
        integrity: SourceIntegrity::ChecksummedOnly,
        source_id: 7,
        source_incarnation: SourceIncarnation::new([0x3C; 16]),
        source_epoch: 2,
        sequence: 9,
        acquired_at_ns: 5_000_000,
        clock: MeasurementClock::HostMonotonic,
    }
}

fn with_variation(variation: Option<MagneticVariationSample>) -> wire::AvionicsState {
    super::super::avionics_to_wire(AvionicsSample {
        attitude: None,
        kinematics: None,
        baro: None,
        geodetic: None,
        magnetic_variation: variation,
        estimator_status_stamp: None,
        valid_flags: 0,
        quality: 0,
    })
}

fn variation(source: MagneticVariationSource, declination_rad: f32) -> MagneticVariationSample {
    MagneticVariationSample {
        declination_rad,
        source,
        model_id: None,
        model_epoch: None,
        stamp: estimate_stamp(),
    }
}

#[test]
fn a_stated_variation_crosses_with_its_source_model_and_stamp() {
    let sample = MagneticVariationSample {
        model_id: Some("WMM2025"),
        model_epoch: Some(2025.0),
        ..variation(MagneticVariationSource::FcModel, -0.2113)
    };
    let avionics = with_variation(Some(sample));
    let wire_variation = avionics.magnetic_variation.expect("the variation crosses");
    assert_eq!(wire_variation.declination_rad, -0.2113);
    assert_eq!(
        wire_variation.source,
        wire::MagneticVariationSource::FcModel as i32
    );
    assert_eq!(wire_variation.model_id, "WMM2025");
    assert_eq!(wire_variation.model_epoch, 2025.0);
    let stamp = avionics
        .magnetic_variation_stamp
        .expect("its stamp crosses");
    assert_eq!((stamp.source_id, stamp.sequence), (7, 9));
}

#[test]
fn no_stated_variation_is_absent_with_no_stamp() {
    let avionics = with_variation(None);
    assert!(avionics.magnetic_variation.is_none());
    assert!(avionics.magnetic_variation_stamp.is_none());
}

#[test]
fn a_magnetic_heading_crosses_as_source_none_with_no_value() {
    let avionics = with_variation(Some(variation(MagneticVariationSource::None, f32::NAN)));
    let wire_variation = avionics.magnetic_variation.expect("NONE is a statement");
    assert_eq!(
        wire_variation.source,
        wire::MagneticVariationSource::None as i32
    );
    assert_eq!(wire_variation.declination_rad, 0.0);
    assert!(avionics.magnetic_variation_stamp.is_some());
}

#[test]
fn a_non_finite_declination_is_refused_with_its_stamp() {
    for bad in [f32::NAN, f32::INFINITY] {
        let avionics = with_variation(Some(variation(MagneticVariationSource::FcModel, bad)));
        assert!(avionics.magnetic_variation.is_none(), "{bad}");
        assert!(avionics.magnetic_variation_stamp.is_none(), "{bad}");
    }
}
