use super::gimbal_reference;
use pilotage_adapter_api::{GimbalAxisLocks, GimbalYawFrame};

#[test]
fn an_explicit_frame_bit_names_the_yaw_frame() {
    assert_eq!(gimbal_reference(32).yaw_frame, GimbalYawFrame::Vehicle);
    assert_eq!(gimbal_reference(64).yaw_frame, GimbalYawFrame::Earth);
    // A yaw lock does not override an explicit vehicle frame.
    assert_eq!(gimbal_reference(16 | 32).yaw_frame, GimbalYawFrame::Vehicle);
}

#[test]
fn without_a_frame_bit_the_yaw_lock_names_the_frame() {
    assert_eq!(gimbal_reference(0).yaw_frame, GimbalYawFrame::Vehicle);
    assert_eq!(gimbal_reference(16).yaw_frame, GimbalYawFrame::Earth);
}

#[test]
fn each_lock_bit_sets_only_its_own_axis() {
    let table = [
        (
            4,
            GimbalAxisLocks {
                roll: true,
                ..GimbalAxisLocks::default()
            },
        ),
        (
            8,
            GimbalAxisLocks {
                pitch: true,
                ..GimbalAxisLocks::default()
            },
        ),
        (
            16,
            GimbalAxisLocks {
                yaw: true,
                ..GimbalAxisLocks::default()
            },
        ),
        (
            // RETRACT, NEUTRAL, and the RC bits are not locks.
            1 | 2 | 128 | 256 | 512,
            GimbalAxisLocks::default(),
        ),
    ];
    for (flags, locks) in table {
        assert_eq!(gimbal_reference(flags).locks, locks, "flags {flags}");
    }
}
