use super::{BasicFlightCommand, Mini3FlightReport, VirtualStickReport};
use crate::{DumlFrame, DumlParser, RouteAddress};
use turret_dji::{frame_checksum, header_checksum};

fn parse(bytes: &[u8]) -> DumlFrame {
    let mut frames = Vec::new();
    let mut parser = DumlParser::new();
    parser.push(bytes, |frame| frames.push(frame));
    assert_eq!(frames.len(), 1);
    frames.remove(0)
}

fn seal(bytes: &mut [u8]) {
    let length = bytes.len();
    let [low, high] = (length as u16 | (1 << 10)).to_le_bytes();
    bytes[1] = low;
    bytes[2] = high;
    bytes[3] = header_checksum(&bytes[..3]);
    let crc = frame_checksum(&bytes[..length - 2]).to_le_bytes();
    bytes[length - 2..].copy_from_slice(&crc);
}

fn report(command: u8, payload: &[u8]) -> Vec<u8> {
    let mut bytes = vec![0x55, 0, 0, 0, 14, 2, 0xff, 0xff, 0, 3, command];
    bytes.extend_from_slice(payload);
    bytes.extend_from_slice(&[0, 0]);
    seal(&mut bytes);
    bytes
}

#[test]
fn native_actions_use_the_basic_endpoint_and_keep_the_caller_sequence() {
    for sequence in [0, 0x7060, 0x7080, u16::MAX] {
        for (command, value) in [
            (BasicFlightCommand::Takeoff, 1),
            (BasicFlightCommand::Land, 2),
            (BasicFlightCommand::ReturnHome, 6),
            (BasicFlightCommand::CancelLanding, 14),
        ] {
            let packet = command.packet(sequence);
            let frame = parse(packet.bytes());
            assert_eq!(frame.sender(), RouteAddress(2));
            assert_eq!(frame.receiver(), RouteAddress(3));
            assert_eq!(frame.flags(), 0x40);
            assert_eq!(frame.command_set(), 3);
            assert_eq!(frame.command_id(), 0x2a);
            assert_eq!(frame.payload(), [value]);
            assert_eq!(frame.sequence(), sequence);
            assert_eq!(packet.sequence(), sequence);
        }
    }
}

#[test]
fn basic_response_requires_exact_route_sequence_flags_and_single_result() {
    let command = BasicFlightCommand::Land.packet(0x7081);
    let mut bytes = command.bytes().to_vec();
    bytes[4] = 3;
    bytes[5] = 2;
    bytes[8] = 0x80;
    for code in [0, 2, 255] {
        bytes[11] = code;
        seal(&mut bytes);
        assert_eq!(command.response(&parse(&bytes)), Some(code));
    }
    for index in [4, 5, 6, 7, 8, 9, 10] {
        let mut wrong = bytes.clone();
        wrong[index] ^= 1;
        seal(&mut wrong);
        assert_eq!(command.response(&parse(&wrong)), None);
    }
    for length in [13, 15] {
        let mut wrong = bytes.clone();
        wrong.resize(length, 0);
        seal(&mut wrong);
        assert_eq!(command.response(&parse(&wrong)), None);
    }
}

#[test]
fn captured_mini3_report_preserves_motor_and_flight_distinction() {
    let source = include_str!("../adapter/airframe/fixtures/mini3-reported-attitude.hex");
    for line in source.lines() {
        let bytes: Vec<u8> = line
            .as_bytes()
            .as_chunks::<2>()
            .0
            .iter()
            .map(|hex| u8::from_str_radix(core::str::from_utf8(hex).unwrap(), 16).unwrap())
            .collect();
        let frame = parse(&bytes);
        let state = Mini3FlightReport::from_push(&frame).unwrap();
        assert!(!state.motors_running);
        assert!(!state.is_flying);
        assert_eq!(state.mode, 6);
        assert_eq!(state.sequence, frame.sequence());
        assert!(!Mini3FlightReport::reports_other_model(&frame));
    }
}

#[test]
fn captured_basic_action_rejections_match_only_their_request() {
    let takeoff = BasicFlightCommand::Takeoff.packet(0x7080);
    let land = BasicFlightCommand::Land.packet(0x7081);
    // Complete FC response frames contain no device identifiers or location.
    let takeoff_reply = parse(&[
        0x55, 0x0e, 0x04, 0x66, 0x03, 0x02, 0x80, 0x70, 0x80, 0x03, 0x2a, 0x4a, 0xa9, 0x0d,
    ]);
    let land_reply = parse(&[
        0x55, 0x0e, 0x04, 0x66, 0x03, 0x02, 0x81, 0x70, 0x80, 0x03, 0x2a, 0x02, 0xce, 0xc7,
    ]);
    assert_eq!(takeoff.response(&takeoff_reply), Some(74));
    assert_eq!(land.response(&land_reply), Some(2));
    assert_eq!(takeoff.response(&land_reply), None);
    assert_eq!(land.response(&takeoff_reply), None);
}

#[test]
fn synthetic_flag_combinations_do_not_treat_running_motors_as_flight() {
    for flags in 0..=255 {
        let mut payload = [0; 70];
        payload[48] = 87;
        payload[32] = flags;
        payload[30] = 0x86;
        let state = Mini3FlightReport::from_push(&parse(&report(0x43, &payload))).unwrap();
        assert_eq!(state.motors_running, flags & 8 != 0);
        assert_eq!(state.is_flying, flags & 14 == 12);
        assert_eq!(state.mode, 6);
    }
}

#[test]
fn model_length_and_route_must_match_the_mini3_report() {
    let mut payload = [0; 70];
    payload[48] = 87;
    let valid = report(0x43, &payload);
    for index in [4, 5, 8, 9, 10] {
        let mut bytes = valid.clone();
        bytes[index] ^= 1;
        seal(&mut bytes);
        assert!(Mini3FlightReport::from_push(&parse(&bytes)).is_none());
        assert!(!Mini3FlightReport::reports_other_model(&parse(&bytes)));
    }
    for model in [0, 86, 88, 255] {
        payload[48] = model;
        let frame = parse(&report(0x43, &payload));
        assert!(Mini3FlightReport::from_push(&frame).is_none());
        assert!(Mini3FlightReport::reports_other_model(&frame));
    }
    payload[48] = 87;
    for length in [69, 71] {
        let mut wrong = payload.to_vec();
        wrong.resize(length, 0);
        assert!(Mini3FlightReport::from_push(&parse(&report(0x43, &wrong))).is_none());
    }
}

#[test]
fn virtual_stick_report_is_a_distinct_enable_flag() {
    for flags in [0, 1, 2, 3, 0xfd, 0xff] {
        let mut payload = [0; 78];
        payload[32] = flags;
        let frame = parse(&report(0x44, &payload));
        let state = VirtualStickReport::from_push(&frame).unwrap();
        assert_eq!(state.enabled, flags & 2 != 0);
        assert_eq!(state.sequence, u16::MAX);
        assert!(Mini3FlightReport::from_push(&frame).is_none());
    }
    for length in [70, 77, 79] {
        assert!(VirtualStickReport::from_push(&parse(&report(0x44, &vec![0; length]))).is_none());
    }
    for index in [4, 5, 8, 9, 10] {
        let mut bytes = report(0x44, &[0; 78]);
        bytes[index] ^= 1;
        seal(&mut bytes);
        assert!(VirtualStickReport::from_push(&parse(&bytes)).is_none());
    }
}
