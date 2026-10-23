use navigate_contract::MonotonicNanos;
use pilotage_adapter_api::{
    AvionicsAttitudeSample, MagneticVariationSample, MagneticVariationSource, SourceRole,
    TelemetrySample,
};
use pilotage_mission::OwnshipAltitude;
use pilotage_protocol::HeadingReference;
use pilotage_timing::MonoTimestamp;
use pilotage_world::{MagneticEpoch, WMM2025};

use super::super::mission_estimate::{dji_shaped, one_vehicle_sample};
use super::super::sample_to_wire;
use super::{MagneticCorrection, turned_about_down};
use crate::runtime::automation::ownship::estimate_from_wire;

const YAW: f32 = 0.3;

fn epoch() -> MagneticEpoch {
    MagneticEpoch::from_decimal_year(2026.8)
}

fn attitude(sample: &TelemetrySample) -> ([f32; 4], pilotage_adapter_api::MeasurementStamp) {
    match sample
        .avionics
        .as_ref()
        .and_then(|avionics| avionics.attitude)
    {
        Some(AvionicsAttitudeSample::Canonical {
            quat_wxyz, stamp, ..
        }) => (quat_wxyz, stamp),
        other => panic!("the sample carries a canonical attitude, not {other:?}"),
    }
}

fn yaw_of(sample: &TelemetrySample) -> f32 {
    let ([w, _, _, z], _) = attitude(sample);
    2.0 * z.atan2(w)
}

fn variation(sample: &TelemetrySample) -> Option<MagneticVariationSample> {
    sample.avionics.as_ref()?.magnetic_variation
}

#[test]
fn a_declared_magnetic_heading_turns_true_by_the_model_at_the_fix() {
    let mut sample = one_vehicle_sample(dji_shaped(YAW));
    let fix = sample
        .avionics
        .as_ref()
        .and_then(|avionics| avionics.geodetic)
        .expect("a fix")
        .position;
    let expected =
        pilotage_world::magnetic_field_at(fix.latitude_deg, fix.longitude_deg, 0.0, epoch())
            .expect("inside the model's domain")
            .declination_rad() as f32;
    assert!(expected != 0.0, "the model has a declination here");

    let mut correction = MagneticCorrection::at(epoch());
    let applied = correction
        .apply(&mut sample, Some(HeadingReference::Magnetic))
        .expect("turned");
    assert_eq!(applied, expected);
    assert!((yaw_of(&sample) - (YAW + expected)).abs() < 1e-5);
    let stated = variation(&sample).expect("the variation applied is stated");
    assert_eq!(stated.source, MagneticVariationSource::SharedModel);
    assert_eq!(stated.declination_rad, expected);
    assert_eq!(stated.model_id, Some(WMM2025.dataset.name));
    assert_eq!(stated.stamp.role, SourceRole::OperationalEstimate);

    // The turned heading is the one the mission estimate flies.
    let wire_sample = sample_to_wire(sample, MonoTimestamp::from_nanos(2_000));
    assert!(
        wire_sample
            .avionics
            .as_ref()
            .is_some_and(|avionics| avionics.magnetic_variation.is_some())
    );
    let ownship = estimate_from_wire(
        &wire_sample,
        MonotonicNanos::from_nanos(2_000),
        |_, _, altitude| match altitude {
            OwnshipAltitude::AboveAnchor(height_m) => [0.0, 0.0, -height_m],
            OwnshipAltitude::Ellipsoid(altitude_m) => [0.0, 0.0, -altitude_m - 1_000.0],
        },
    )
    .expect("admitted");
    assert!((ownship.yaw_rad.expect("heading") - f64::from(YAW + expected)).abs() < 1e-5);
}

#[test]
fn only_a_magnetic_declaration_without_a_stated_variation_is_turned() {
    for reference in [None, Some(HeadingReference::True)] {
        let mut sample = one_vehicle_sample(dji_shaped(YAW));
        assert!(
            MagneticCorrection::at(epoch())
                .apply(&mut sample, reference)
                .is_none()
        );
        assert!((yaw_of(&sample) - YAW).abs() < 1e-7);
        assert!(variation(&sample).is_none());
    }
    // The vehicle's own statement stands.
    let mut own = one_vehicle_sample(dji_shaped(YAW));
    let (_, stamp) = attitude(&own);
    own.avionics.as_mut().expect("avionics").magnetic_variation = Some(MagneticVariationSample {
        declination_rad: 0.05,
        source: MagneticVariationSource::FcModel,
        model_id: None,
        model_epoch: None,
        stamp,
    });
    assert!(
        MagneticCorrection::at(epoch())
            .apply(&mut own, Some(HeadingReference::Magnetic))
            .is_none()
    );
    assert!((yaw_of(&own) - YAW).abs() < 1e-7);
    assert_eq!(variation(&own).map(|v| v.declination_rad), Some(0.05));
    // Without a fix there is nowhere to evaluate the model.
    let mut unfixed = one_vehicle_sample(dji_shaped(YAW));
    unfixed.avionics.as_mut().expect("avionics").geodetic = None;
    assert!(
        MagneticCorrection::at(epoch())
            .apply(&mut unfixed, Some(HeadingReference::Magnetic))
            .is_none()
    );
    assert!((yaw_of(&unfixed) - YAW).abs() < 1e-7);
    assert!(variation(&unfixed).is_none());
    // A date outside the model's validity leaves the heading as reported.
    let mut dated = one_vehicle_sample(dji_shaped(YAW));
    assert!(
        MagneticCorrection::at(MagneticEpoch::from_decimal_year(2040.0))
            .apply(&mut dated, Some(HeadingReference::Magnetic))
            .is_none()
    );
    assert!((yaw_of(&dated) - YAW).abs() < 1e-7);
}

#[test]
fn turning_about_down_moves_only_the_heading() {
    // A body pitched 0.2 down at a heading of 0.3, turned right by 0.5.
    let (sp, cp) = (-0.2_f32 / 2.0).sin_cos();
    let (sy, cy) = (YAW / 2.0).sin_cos();
    let pitched = [cp * cy, -sp * sy, sp * cy, cp * sy];
    let [w, x, y, z] = turned_about_down(pitched, 0.5);
    let yaw = (2.0 * (w * z + x * y)).atan2(1.0 - 2.0 * (y * y + z * z));
    let pitch = (2.0 * (w * y - z * x)).clamp(-1.0, 1.0).asin();
    assert!((yaw - (YAW + 0.5)).abs() < 1e-6);
    assert!((pitch + 0.2).abs() < 1e-6);
    let norm = (w * w + x * x + y * y + z * z).sqrt();
    assert!((norm - 1.0).abs() < 1e-6);
}
