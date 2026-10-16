#![allow(clippy::expect_used, clippy::panic)]

use super::{sample, session, with_default};
use crate::plan::ControlPlan;
use crate::sample::Mode;

#[test]
fn input_loss_emits_no_publishable_plan_and_consumes_edges() {
    let mut runtime = with_default();
    let live = session(Mode::QuadPilot, true);
    runtime.evaluate(&sample(&[0.0; 4], &[]), &live);

    let mut lost = live;
    lost.input_lost = true;
    assert_eq!(
        runtime.evaluate(&sample(&[0.0; 4], &[]), &lost),
        ControlPlan::default()
    );

    let pressed = runtime.evaluate(&sample(&[0.0; 4], &[9]), &lost);
    assert!(pressed.arm_suppressed);
    assert!(pressed.motion.is_none());
    assert!(pressed.gimbal.is_none());
    assert!(pressed.lease.is_none());
    assert!(pressed.motion_lease.is_none());
    assert!(!pressed.arm);
    assert!(!pressed.disarm);

    let held = runtime.evaluate(&sample(&[0.0; 4], &[9]), &live);
    assert!(
        !held.arm,
        "a press consumed during input loss cannot re-fire"
    );
}
