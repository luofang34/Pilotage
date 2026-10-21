use super::*;
use crate::DumlParser;

fn parse(bytes: &[u8]) -> DumlFrame {
    let mut frames = Vec::new();
    DumlParser::new().push(bytes, |frame| frames.push(frame));
    assert_eq!(frames.len(), 1);
    frames.remove(0)
}

#[test]
fn mode_packets_use_the_acknowledged_basic_fc_route_and_exact_response() {
    for (mode, value) in [
        (VirtualStickMode::Enable, 1),
        (VirtualStickMode::Disable, 2),
    ] {
        let packet = mode.packet(0xfffe);
        let frame = parse(packet.bytes());
        assert_eq!(
            (frame.sender(), frame.receiver()),
            (RouteAddress(2), RouteAddress(3))
        );
        assert_eq!(
            (frame.flags(), frame.command_set(), frame.command_id()),
            (0x40, 3, 0x80)
        );
        assert_eq!(frame.payload(), [value]);
        let mut response = *packet.bytes();
        response[4] = 3;
        response[5] = 2;
        response[8] = 0x80;
        for code in [0, 1, 255] {
            response[11] = code;
            seal(&mut response);
            assert_eq!(packet.response(&parse(&response)), Some(code));
        }
        for index in [4, 5, 6, 7, 8, 9, 10] {
            let mut wrong = response;
            wrong[index] ^= 1;
            seal(&mut wrong);
            assert_eq!(packet.response(&parse(&wrong)), None);
        }
    }
}

#[test]
fn body_frd_maps_to_forward_right_up_and_clockwise_degrees_without_ack_flag() {
    let input = VirtualStickVelocity::new(0.6, -0.8, -0.5, core::f32::consts::FRAC_PI_6).unwrap();
    let frame = parse(&input.packet(0xffff));
    assert_eq!(
        (frame.sender(), frame.receiver()),
        (RouteAddress(2), RouteAddress(0xc3))
    );
    assert_eq!(
        (frame.flags(), frame.command_set(), frame.command_id()),
        (0, 3, 0x8e)
    );
    assert_eq!(frame.sequence(), 0xffff);
    assert_eq!(frame.payload()[0], 0x4b);
    let decoded: Vec<f32> = frame.payload()[1..]
        .as_chunks::<4>()
        .0
        .iter()
        .map(|bytes| f32::from_le_bytes(*bytes))
        .collect();
    assert_eq!(&decoded[..3], &[0.6, -0.8, 0.5]);
    assert!((decoded[3] - 30.0).abs() < 0.00001);
    let reverse = parse(
        &VirtualStickVelocity::new(-1.0, 0.0, 0.5, -0.2)
            .unwrap()
            .packet(0),
    );
    assert_eq!(
        f32::from_le_bytes(reverse.payload()[9..13].try_into().unwrap()),
        -0.5
    );
    assert!(f32::from_le_bytes(reverse.payload()[13..17].try_into().unwrap()) < 0.0);
}

#[test]
fn encoder_rejects_nonfinite_and_excessive_vector_norm() {
    for input in [
        [1.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 0.501, 0.0],
        [0.0, 0.0, 0.0, 0.524],
        [f32::NAN, 0.0, 0.0, 0.0],
        [0.0, f32::INFINITY, 0.0, 0.0],
        [0.0, 0.0, f32::NEG_INFINITY, 0.0],
    ] {
        let [x, y, z, yaw] = input;
        assert!(VirtualStickVelocity::new(x, y, z, yaw).is_err());
    }
}
