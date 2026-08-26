//! What the map is allowed to draw from a telemetry sample.
//!
//! These restate the browser's rules against the same values. Nothing shared
//! enforces the agreement, so the numbers are stated outright here: a reader
//! comparing this against `clients/web/situation-ownship.js` can see in one
//! pass whether the two clients still draw the same mark.

#![allow(clippy::expect_used, clippy::panic)]

use pilotage_protocol::wire;

use super::{MarkMemory, VehicleFix, from_sample};

mod directions;
mod lanes;

/// Reads a sample the way a client that has just connected reads its first
/// one: nothing seen before, so every group states a new measurement.
fn first_sample(sample: &wire::TelemetrySample) -> Option<VehicleFix> {
    from_sample(&mut MarkMemory::default(), sample, 0)
}

/// Bits 0 and 3: attitude and velocity stated.
const ATTITUDE_AND_VELOCITY: u32 = 0b1001;

fn stamp() -> wire::MeasurementStamp {
    wire::MeasurementStamp {
        sequence: 1,
        ..Default::default()
    }
}

/// The stamp the truth lane is gated on: the simulator's oracle saying so.
fn truth_stamp(sequence: u32) -> wire::MeasurementStamp {
    wire::MeasurementStamp {
        role: 2,
        sequence,
        ..Default::default()
    }
}

/// The stamp the estimate lane's fix is gated on.
fn estimate_fix_stamp(sequence: u32) -> wire::MeasurementStamp {
    wire::MeasurementStamp {
        role: 1,
        sequence,
        ..Default::default()
    }
}

fn fix() -> wire::GeodeticFix {
    wire::GeodeticFix {
        latitude_deg: 47.397_742,
        longitude_deg: 8.545_594,
        horizontal_datum: 1,
        ..Default::default()
    }
}

/// A truth lane facing north at `speed`, level.
fn truth(speed_north: f32, speed_east: f32) -> wire::TelemetrySample {
    wire::TelemetrySample {
        sim_truth: Some(Box::new(wire::SimTruthState {
            quat_w: 1.0,
            quat_x: 0.0,
            quat_y: 0.0,
            quat_z: 0.0,
            vel_n_mps: speed_north,
            vel_e_mps: speed_east,
            valid_flags: ATTITUDE_AND_VELOCITY,
            stamp: Some(truth_stamp(1)),
            geodetic: Some(fix()),
            ..Default::default()
        })),
        ..Default::default()
    }
}

/// A stamp stating measurement `sequence`, so successive values read as
/// successive measurements of the same group.
fn stamp_seq(sequence: u32) -> wire::MeasurementStamp {
    wire::MeasurementStamp {
        sequence,
        ..Default::default()
    }
}

/// An estimate lane stating a position, an attitude and a velocity, each
/// group stamped separately so one can be frozen while another advances.
fn estimate(attitude: u32, kinematics: u32, geodetic: u32) -> wire::TelemetrySample {
    wire::TelemetrySample {
        avionics: Some(wire::AvionicsState {
            quat_w: 1.0,
            vel_n_mps: 3.0,
            valid_flags: ATTITUDE_AND_VELOCITY,
            geodetic: Some(fix()),
            geodetic_stamp: Some(estimate_fix_stamp(geodetic)),
            attitude_stamp: Some(stamp_seq(attitude)),
            kinematics_stamp: Some(stamp_seq(kinematics)),
            estimator_status_stamp: Some(stamp()),
            ..Default::default()
        }),
        ..Default::default()
    }
}

/// An estimate lane moving north at `speed`, every group freshly stamped.
fn moving_at(speed: f32, sequence: u32) -> wire::TelemetrySample {
    let mut sample = estimate(sequence, sequence, sequence);
    sample.avionics.as_mut().expect("estimate lane").vel_n_mps = speed;
    sample
}

/// One estimate sample whose position is `edit`ed before it is read.
fn with_position(edit: impl FnOnce(&mut wire::GeodeticFix)) -> Option<VehicleFix> {
    let mut sample = estimate(1, 1, 1);
    edit(
        sample
            .avionics
            .as_mut()
            .expect("estimate lane")
            .geodetic
            .as_mut()
            .expect("a position"),
    );
    first_sample(&sample)
}
