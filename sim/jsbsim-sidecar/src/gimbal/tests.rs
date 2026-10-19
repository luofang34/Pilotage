//! The gimbal maps PX4's outputs back to angles and follows them.

use super::{Gimbal, GimbalLimits};

/// The x500 gimbal's limits: PX4 airframe 4019's parameters.
fn x500() -> GimbalLimits {
    GimbalLimits::from_px4(180.0, 45.0, -135.0, 720.0)
}

fn settle(gimbal: &mut Gimbal, outputs: [f64; 3]) {
    for _ in 0..1_000 {
        gimbal.step(Some(outputs), 0.004);
    }
}

#[test]
fn outputs_map_to_angles_with_pxs_limits() {
    let mut gimbal = Gimbal::new(x500());
    settle(&mut gimbal, [0.5, -1.0 / 3.0, 0.25]);
    let [roll, pitch, yaw] = gimbal.angles_deg();
    assert!((roll - 45.0).abs() < 1e-6, "{roll}");
    // Down is the long side of pitch: -1/3 of -135 is -45.
    assert!((pitch + 45.0).abs() < 1e-6, "{pitch}");
    assert!((yaw - 90.0).abs() < 1e-6, "{yaw}");
    settle(&mut gimbal, [-1.0, 1.0, -1.0]);
    assert_eq!(
        gimbal.angles_deg().map(|a| a.round()),
        [-90.0, 45.0, -360.0]
    );
}

#[test]
fn the_gimbal_lags_and_holds_when_disarmed() {
    let mut gimbal = Gimbal::new(x500());
    gimbal.step(Some([0.0, -1.0, 0.0]), 0.05);
    let pitch = gimbal.angles_deg()[1];
    // One time constant: 63% of the way to -135.
    assert!((pitch / -135.0 - 0.632).abs() < 0.01, "{pitch}");
    settle(&mut gimbal, [0.0, -1.0, 0.0]);
    gimbal.step(None, 10.0);
    assert!((gimbal.angles_deg()[1] + 135.0).abs() < 1e-6);
    gimbal.step(Some([f64::NAN; 3]), 10.0);
    assert!(gimbal.angles_deg().iter().all(|a| a.is_finite()));
}
