#![allow(clippy::expect_used, clippy::panic)]

use super::{
    DecodeError, Decoder, Direction, EncodeError, Frame, MAX_PAYLOAD, Version, crc8_dvb_s2,
};

fn hex(text: &str) -> Vec<u8> {
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).expect("fixture hex"))
        .collect()
}

#[test]
fn crc8_dvb_s2_matches_the_standard_check_value() {
    // The CRC catalogue check value: CRC-8/DVB-S2 over "123456789".
    let crc = b"123456789".iter().fold(0, |crc, &b| crc8_dvb_s2(crc, b));
    assert_eq!(crc, 0xBC);
}

#[test]
fn v1_status_request_matches_the_canonical_bytes() {
    let bytes = Frame::request(Version::V1, 101, vec![])
        .encode()
        .expect("encodes");
    assert_eq!(bytes, hex("244d3c006565"));
}

#[test]
fn v1_checksum_covers_size_command_and_payload() {
    let bytes = Frame::request(Version::V1, 230, vec![0x96, 0x6c])
        .encode()
        .expect("encodes");
    assert_eq!(bytes[..7], hex("244d3c02e6966c")[..]);
    assert_eq!(bytes[7], 0x02 ^ 0xe6 ^ 0x96 ^ 0x6c);
}

#[test]
fn v2_request_carries_a_sixteen_bit_command_and_crc() {
    let bytes = Frame::request(Version::V2, 0x1F01, vec![0xAA])
        .encode()
        .expect("encodes");
    assert_eq!(bytes[..8], hex("24583c00011f0100")[..]);
    assert_eq!(bytes[8], 0xAA);
    let crc = bytes[3..9].iter().fold(0, |crc, &b| crc8_dvb_s2(crc, b));
    assert_eq!(bytes[9], crc);
}

#[test]
fn a_captured_betaflight_api_version_reply_decodes() {
    // Betaflight SITL (API 1.49) reply to MSP_API_VERSION.
    let frames = Decoder::new().feed(&hex("244d3e030100013132"));
    assert_eq!(
        frames,
        vec![Ok(Frame {
            version: Version::V1,
            direction: Direction::Response,
            command: 1,
            payload: vec![0x00, 0x01, 0x31],
        })]
    );
}

#[test]
fn every_encoded_frame_decodes_back_to_itself() {
    for version in [Version::V1, Version::V2] {
        for direction in [Direction::Request, Direction::Response, Direction::Error] {
            let frame = Frame {
                version,
                direction,
                command: 108,
                payload: (0u8..40).collect(),
            };
            let bytes = frame.encode().expect("encodes");
            assert_eq!(Decoder::new().feed(&bytes), vec![Ok(frame)]);
        }
    }
}

#[test]
fn bytes_outside_frames_are_skipped_and_counted() {
    let mut stream = b"# CLI text\r\n".to_vec();
    stream.extend(hex("244d3e066c0000000000006a"));
    let mut decoder = Decoder::new();
    let frames = decoder.feed(&stream);
    assert_eq!(frames.len(), 1);
    assert!(matches!(&frames[0], Ok(frame) if frame.command == 108));
    assert_eq!(decoder.skipped(), 12, "the CLI text, not the frame");
}

#[test]
fn a_corrupt_checksum_is_reported_and_the_next_frame_still_decodes() {
    let mut stream = hex("244d3e066c0000000000006b");
    stream.extend(hex("244d3e066d0000000000006b"));
    let frames = Decoder::new().feed(&stream);
    assert_eq!(
        frames[0],
        Err(DecodeError::Checksum {
            version: Version::V1,
            command: 108,
            expected: 0x6a,
            actual: 0x6b,
        })
    );
    assert!(matches!(&frames[1], Ok(frame) if frame.command == 109));
}

#[test]
fn a_v2_frame_with_a_flipped_payload_bit_fails_its_crc() {
    let mut bytes = Frame::request(Version::V2, 150, vec![1, 2, 3])
        .encode()
        .expect("encodes");
    bytes[9] ^= 0x01;
    assert!(matches!(
        Decoder::new().feed(&bytes)[..],
        [Err(DecodeError::Checksum {
            version: Version::V2,
            command: 150,
            ..
        })]
    ));
}

#[test]
fn a_v1_jumbo_frame_decodes() {
    let payload: Vec<u8> = (0..300u16).map(|i| (i % 251) as u8).collect();
    let mut bytes = vec![b'$', b'M', b'>', 255, 230, 0x2C, 0x01];
    bytes.extend(&payload);
    let checksum = payload
        .iter()
        .fold(255 ^ 230 ^ 0x2C ^ 0x01, |acc, b| acc ^ b);
    bytes.push(checksum);
    let frames = Decoder::new().feed(&bytes);
    assert!(matches!(&frames[..], [Ok(frame)] if frame.payload == payload));
}

#[test]
fn an_oversize_v2_header_is_refused_without_buffering() {
    let declared = u16::try_from(MAX_PAYLOAD + 1).expect("fits");
    let mut bytes = vec![b'$', b'X', b'>', 0, 150, 0];
    bytes.extend(declared.to_le_bytes());
    assert_eq!(
        Decoder::new().feed(&bytes),
        vec![Err(DecodeError::Oversize {
            command: 150,
            declared: MAX_PAYLOAD + 1,
        })]
    );
}

#[test]
fn v1_refuses_what_it_cannot_frame() {
    assert_eq!(
        Frame::request(Version::V1, 0x1001, vec![]).encode(),
        Err(EncodeError::CommandOutOfRange { command: 0x1001 })
    );
    assert!(matches!(
        Frame::request(Version::V1, 1, vec![0; 255]).encode(),
        Err(EncodeError::PayloadTooLong { len: 255, .. })
    ));
}

#[test]
fn frames_split_across_reads_reassemble() {
    let bytes = hex("244d3e030100013132");
    let mut decoder = Decoder::new();
    let mut frames = Vec::new();
    for byte in bytes {
        frames.extend(decoder.push(byte));
    }
    assert_eq!(frames.len(), 1);
}

#[test]
fn a_captured_betaflight_v2_exchange_round_trips() {
    // MSP_API_VERSION over v2, and the SITL's v2 reply.
    let request = Frame::request(Version::V2, 1, vec![])
        .encode()
        .expect("encodes");
    assert_eq!(request, hex("24583c000100000045"));
    let frames = Decoder::new().feed(&hex("24583e0001000300000131b3"));
    assert_eq!(
        frames,
        vec![Ok(Frame {
            version: Version::V2,
            direction: Direction::Response,
            command: 1,
            payload: vec![0x00, 0x01, 0x31],
        })]
    );
}
