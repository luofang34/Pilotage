#![allow(clippy::expect_used, clippy::panic)]

use pilotage_protocol::{CommandOutcome, GimbalYawFrame, wire};

use super::{
    ArmState, CommandVerdict, EstimateQuality, EstimatorValidity, command_verdict, gimbal_reference,
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
