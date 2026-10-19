#![allow(clippy::expect_used, clippy::panic)]

use super::{
    FDM_PACKET_LEN, FdmPacket, PACKET_TIME_STEP_S, PacketClock, accelerometer_field,
    attitude_from_field, decode_servo_packet, gyro_field, orientation_field,
};
use crate::geodesy::Origin;
use crate::quad::{GRAVITY, QuadState};
use crate::rotation::{Quat, Vec3};

const ORIGIN: Origin = Origin {
    lat_deg: 47.397_742,
    lon_deg: 8.545_594,
    alt_m: 488.0,
};

/// FRD <-> FLU and NED <-> NWU are both `(x, -y, -z)`.
fn flip(v: Vec3) -> Vec3 {
    Vec3::new(v.x, -v.y, -v.z)
}

fn sample_attitudes() -> Vec<Quat> {
    let mut out = Vec::new();
    for roll in [-0.7, 0.0, 0.4] {
        for pitch in [-0.5, 0.0, 0.3] {
            for yaw in [-2.5, 0.0, 1.2, 3.0] {
                out.push(Quat::from_euler(roll, pitch, yaw));
            }
        }
    }
    out
}

#[test]
fn the_packet_quaternion_is_the_flu_to_nwu_rotation() {
    let v_frd = Vec3::new(0.3, -1.2, 0.8);
    for q in sample_attitudes() {
        let field = orientation_field(q);
        let packet = Quat {
            w: field[0],
            x: field[1],
            y: field[2],
            z: field[3],
        };
        let expected = flip(q.rotate(v_frd));
        let got = packet.rotate(flip(v_frd));
        assert!((got - expected).norm() < 1e-12, "{q:?}");
    }
}

#[test]
fn orientation_round_trips() {
    for q in sample_attitudes() {
        let back = attitude_from_field(orientation_field(q));
        assert_eq!(back, q);
    }
}

#[test]
fn a_resting_accelerometer_field_reads_down_so_betaflight_reads_up() {
    let field = accelerometer_field(Vec3::new(0.0, 0.0, -GRAVITY));
    assert_eq!(field, [0.0, 0.0, -GRAVITY]);
    // sitl.c negates the field: +1 g on Betaflight's (up) z axis.
    assert_eq!(-field[2], GRAVITY);
    // A forward specific force is +x FLU, so the field carries -x.
    assert_eq!(accelerometer_field(Vec3::new(2.0, 0.0, 0.0))[0], -2.0);
    // A rightward one is -y FLU, so the field carries +y.
    assert_eq!(accelerometer_field(Vec3::new(0.0, 2.0, 0.0))[1], 2.0);
}

#[test]
fn gyro_field_is_the_frd_rate() {
    assert_eq!(gyro_field(Vec3::new(0.1, -0.2, 0.3)), [0.1, -0.2, 0.3]);
}

#[test]
fn the_state_maps_to_geodetic_enu_and_pressure_fields() {
    let mut state = QuadState::landed(Vec3::new(100.0, -50.0, 0.0), 0.0);
    state.position.z = -10.0;
    state.velocity = Vec3::new(1.0, 2.0, -0.5);
    let packet = FdmPacket::from_state(12.5, &state, &ORIGIN);
    assert_eq!(packet.timestamp, 12.5);
    assert_eq!(packet.velocity, [2.0, 1.0, 0.5], "east, north, up");
    let [lon, lat, alt] = packet.position;
    assert!(lat > ORIGIN.lat_deg && lon < ORIGIN.lon_deg);
    assert!((alt - 498.0).abs() < 1e-9);
    let (north, east, down) = ORIGIN.to_local(lat, lon, alt);
    assert!((north - 100.0).abs() < 1e-6 && (east + 50.0).abs() < 1e-6);
    assert!((down + 10.0).abs() < 1e-9);
    assert!(
        (packet.pressure - 95_480.0).abs() < 50.0,
        "{}",
        packet.pressure
    );
}

#[test]
fn packet_bytes_are_eighteen_little_endian_doubles_in_struct_order() {
    let state = QuadState::landed(Vec3::ZERO, 0.0);
    let packet = FdmPacket::from_state(1.5, &state, &ORIGIN);
    let bytes = packet.to_bytes();
    assert_eq!(bytes.len(), FDM_PACKET_LEN);
    let field = |index: usize| {
        f64::from_le_bytes(bytes[index * 8..index * 8 + 8].try_into().expect("8 bytes"))
    };
    assert_eq!(field(0), 1.5);
    assert_eq!(field(6), -GRAVITY, "accelerometer z");
    assert_eq!(field(7), 1.0, "quaternion w");
    assert_eq!(field(14), ORIGIN.lon_deg, "position x is longitude");
    assert_eq!(field(15), ORIGIN.lat_deg);
    assert_eq!(field(17), packet.pressure);
}

#[test]
fn servo_packets_decode_and_other_lengths_do_not() {
    let mut datagram = Vec::new();
    for value in [0.0f32, 0.25, 0.5, 1.0] {
        datagram.extend(value.to_le_bytes());
    }
    assert_eq!(decode_servo_packet(&datagram), Some([0.0, 0.25, 0.5, 1.0]));
    assert_eq!(decode_servo_packet(&datagram[..12]), None);
    datagram.push(0);
    assert_eq!(decode_servo_packet(&datagram), None);
}

#[test]
fn packet_time_stays_out_of_the_sitl_rate_estimate() {
    // sitl.c estimates its clock rate only from packets under 20 ms apart.
    let mut clock = PacketClock::default();
    let mut last = clock.tick();
    assert!(last > 0.0);
    for _ in 0..100_000 {
        let next = clock.tick();
        assert!(next - last >= 0.02, "spacing {}", next - last);
        last = next;
    }
    assert_eq!(PACKET_TIME_STEP_S, 0.025);
}
