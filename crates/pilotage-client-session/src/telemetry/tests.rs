#![allow(clippy::expect_used, clippy::panic)]

use pilotage_protocol::{
    CommandOutcome, GimbalYawFrame, HeadingReference, MagneticVariationSource, wire,
};

use super::{
    ArmState, CommandVerdict, EstimateQuality, EstimatorValidity, command_verdict,
    gimbal_reference, magnetic_variation,
};

#[test]
fn each_validity_bit_authorizes_only_its_own_group() {
    assert_eq!(
        EstimatorValidity::from_flags(0),
        EstimatorValidity::default()
    );
    let all = EstimatorValidity::from_flags(0b1111);
    assert!(all.attitude && all.rates && all.position && all.velocity);
    let position = EstimatorValidity::from_flags(4);
    assert_eq!(
        position,
        EstimatorValidity {
            position: true,
            ..EstimatorValidity::default()
        }
    );
    assert_eq!(
        EstimatorValidity::from_flags(0xFFFF_FFF0),
        EstimatorValidity::default(),
        "high bits authorize nothing"
    );
}

#[test]
fn quality_and_arm_codes_fail_closed() {
    assert_eq!(EstimateQuality::from_code(0), Some(EstimateQuality::Good));
    assert_eq!(
        EstimateQuality::from_code(2),
        Some(EstimateQuality::Unusable)
    );
    assert_eq!(EstimateQuality::from_code(3), None);
    assert_eq!(ArmState::from_code(0), None, "0 is unknown");
    assert_eq!(ArmState::from_code(2), Some(ArmState::Armed));
    assert_eq!(ArmState::from_code(7), None);
}

#[test]
fn a_verdict_needs_a_command_kind_and_never_reads_unknown_as_accepted() {
    let none = wire::FcState::default();
    assert_eq!(command_verdict(&none), None);
    let refused = wire::FcState {
        last_command_kind: 1,
        last_command_outcome: wire::CommandOutcome::Denied as i32,
        ..wire::FcState::default()
    };
    assert_eq!(
        command_verdict(&refused),
        Some(CommandVerdict {
            arm: true,
            outcome: Some(CommandOutcome::Denied),
        })
    );
    let unknown = wire::FcState {
        last_command_kind: 2,
        ..wire::FcState::default()
    };
    assert_eq!(
        command_verdict(&unknown),
        Some(CommandVerdict {
            arm: false,
            outcome: None,
        })
    );
}

#[test]
fn a_gimbal_report_without_a_yaw_frame_has_no_reference() {
    assert_eq!(gimbal_reference(&wire::GimbalAttitude::default()), None);
    let earth = wire::GimbalAttitude {
        yaw_frame: wire::GimbalYawFrame::Earth as i32,
        yaw_locked: true,
        ..wire::GimbalAttitude::default()
    };
    let reference = gimbal_reference(&earth).expect("a frame is named");
    assert_eq!(reference.yaw_frame, GimbalYawFrame::Earth);
    assert!(reference.locks.yaw && !reference.locks.roll && !reference.locks.pitch);
}

fn stated(source: wire::MagneticVariationSource, declination_rad: f32) -> wire::AvionicsState {
    wire::AvionicsState {
        magnetic_variation: Some(wire::MagneticVariation {
            declination_rad,
            source: source as i32,
            model_id: String::new(),
            model_epoch: 0.0,
        }),
        magnetic_variation_stamp: Some(wire::MeasurementStamp::default()),
        ..wire::AvionicsState::default()
    }
}

#[test]
fn a_stated_model_variation_reads_as_a_true_heading_with_its_declination() {
    let mut avionics = stated(wire::MagneticVariationSource::FcModel, -0.2113);
    if let Some(variation) = avionics.magnetic_variation.as_mut() {
        variation.model_id = "WMM2025".to_owned();
        variation.model_epoch = 2025.0;
    }
    let variation = magnetic_variation(&avionics).expect("a stamped, known variation");
    assert_eq!(variation.source, MagneticVariationSource::FcModel);
    assert_eq!(variation.declination_rad, Some(-0.2113));
    assert_eq!(variation.model_id.as_deref(), Some("WMM2025"));
    assert_eq!(variation.model_epoch, Some(2025.0));
    assert_eq!(variation.heading_reference(), HeadingReference::True);
}

#[test]
fn a_vehicle_that_applies_no_variation_reads_as_a_magnetic_heading() {
    let variation = magnetic_variation(&stated(wire::MagneticVariationSource::None, 0.0))
        .expect("NONE is a statement");
    assert_eq!(variation.declination_rad, None);
    assert_eq!(variation.heading_reference(), HeadingReference::Magnetic);
    assert_eq!((variation.model_id, variation.model_epoch), (None, None));
}

#[test]
fn an_unusable_variation_reads_as_none() {
    // No stamp.
    let mut unstamped = stated(wire::MagneticVariationSource::FcModel, 0.1);
    unstamped.magnetic_variation_stamp = None;
    assert_eq!(magnetic_variation(&unstamped), None);
    // An unstated or unknown source.
    assert_eq!(
        magnetic_variation(&stated(wire::MagneticVariationSource::Unspecified, 0.1)),
        None
    );
    let mut unknown = stated(wire::MagneticVariationSource::FcModel, 0.1);
    if let Some(variation) = unknown.magnetic_variation.as_mut() {
        variation.source = 42;
    }
    assert_eq!(magnetic_variation(&unknown), None);
    // An applied declination that is not a number.
    assert_eq!(
        magnetic_variation(&stated(wire::MagneticVariationSource::FcModel, f32::NAN)),
        None
    );
    // Nothing stated at all.
    assert_eq!(magnetic_variation(&wire::AvionicsState::default()), None);
}
