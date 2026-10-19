//! The attitude decomposition and its ANGLE sticks keep their sign at every
//! heading: a held heading never flips the roll or pitch an operator asks for.

#![allow(clippy::expect_used, clippy::panic)]

use pilotage_protocol::{AttitudeThrustIntent, ControlIntent, ReferenceFrame};

use super::demand;
use crate::direct::{DirectDemand, DirectLoop};

/// The ZYX body-to-NED quaternion of `roll`, `pitch`, `heading` (radians),
/// as the client-session attitude builder composes it.
fn attitude(roll: f32, pitch: f32, heading: f32) -> ControlIntent {
    let (sr, cr) = (roll / 2.0).sin_cos();
    let (sp, cp) = (pitch / 2.0).sin_cos();
    let (sy, cy) = (heading / 2.0).sin_cos();
    ControlIntent::AttitudeThrust(AttitudeThrustIntent {
        frame: ReferenceFrame::LocalNed,
        qw: cr * cp * cy + sr * sp * sy,
        qx: sr * cp * cy - cr * sp * sy,
        qy: cr * sp * cy + sr * cp * sy,
        qz: cr * cp * sy - sr * sp * cy,
        thrust: 0.5,
    })
}

#[test]
fn roll_and_pitch_keep_their_sign_at_every_heading() {
    for heading_deg in [0.0_f32, 90.0, 180.0, 270.0] {
        let heading = heading_deg.to_radians();
        for (roll, pitch) in [
            (0.13, 0.0),
            (-0.13, 0.0),
            (0.0, 0.13),
            (0.0, -0.13),
            (0.2, -0.1),
        ] {
            let Some((
                DirectDemand::Attitude {
                    roll: r,
                    pitch: p,
                    heading: h,
                    ..
                },
                _,
            )) = demand(&attitude(roll, pitch, heading))
            else {
                panic!("an attitude demand");
            };
            let at = format!("roll {roll} pitch {pitch} heading {heading_deg}");
            assert!((r - f64::from(roll)).abs() < 1e-5, "{at}: roll {r}");
            assert!((p - f64::from(pitch)).abs() < 1e-5, "{at}: pitch {p}");
            let wrapped = (h - f64::from(heading)).rem_euclid(std::f64::consts::TAU);
            assert!(
                wrapped < 1e-5 || wrapped > std::f64::consts::TAU - 1e-5,
                "{at}: heading {h}"
            );

            // Held at that heading, the sticks are the tilt over the ANGLE
            // limit: right roll is +roll stick, nose-down is +pitch stick.
            let mut direct = DirectLoop::default();
            let demanded = demand(&attitude(roll, pitch, heading)).expect("demand").0;
            let sticks = direct.sticks(demanded, f64::from(heading));
            assert!(
                (sticks.roll - f64::from(roll).to_degrees() / 30.0).abs() < 1e-4,
                "{at}"
            );
            assert!(
                (sticks.pitch + f64::from(pitch).to_degrees() / 30.0).abs() < 1e-4,
                "{at}"
            );
            assert!(sticks.yaw.abs() < 1e-4, "{at}: no yaw at the held heading");
        }
    }
}
