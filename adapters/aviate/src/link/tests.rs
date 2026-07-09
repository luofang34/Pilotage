#![allow(clippy::expect_used, clippy::panic)]

use std::sync::{Arc, Mutex};

use crate::mavlink::AviateMessage;

use super::{LatestAviate, apply_messages};

fn attitude(sysid: u8, qw: f32) -> (u8, AviateMessage) {
    (
        sysid,
        AviateMessage::AttitudeQuaternion {
            time_boot_ms: 1,
            quat_wxyz: [qw, 0.0, 0.0, 0.0],
            rates_rps: [0.0; 3],
        },
    )
}

#[test]
fn locks_onto_the_first_vehicle_and_ignores_the_rest() {
    let state = Arc::new(Mutex::new(LatestAviate::default()));
    // A GCS peer heartbeat must not lock the link.
    apply_messages(&state, &[(255, AviateMessage::Heartbeat)], 0, 0);
    assert!(state.lock().expect("lock").locked_sysid.is_none());

    // First estimate locks; a second vehicle's estimate is ignored.
    apply_messages(&state, &[attitude(1, 0.5), attitude(2, 0.9)], 0, 0);
    let latest = state.lock().expect("lock");
    assert_eq!(latest.locked_sysid, Some(1));
    let att = latest.attitude.expect("attitude cached");
    assert_eq!(att.quat_wxyz[0], 0.5, "vehicle 2's estimate must not win");
}
