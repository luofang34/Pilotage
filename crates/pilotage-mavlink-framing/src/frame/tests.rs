use super::{MAGIC_V2, encode_frame_v2_from};
use crate::compute_crc;

#[test]
fn heartbeat_frame_retains_its_wire_bytes_and_checksum() {
    let payload = [0, 0, 0, 0, 6, 8, 0, 4, 3];
    let mut frame = [0; 21];
    assert_eq!(
        encode_frame_v2_from(4, 0, &payload, 50, 255, 190, &mut frame),
        21
    );
    assert_eq!(
        frame,
        [
            253, 9, 0, 0, 4, 255, 190, 0, 0, 0, 0, 0, 0, 0, 6, 8, 0, 4, 3, 95, 96,
        ]
    );
    assert_eq!(compute_crc(&frame[1..19], 50), 0x605f);
}

#[test]
fn maximum_payload_retains_source_sequence_and_full_message_identifier() {
    let payload = [0xa5; 255];
    let mut frame = [0; 267];
    assert_eq!(
        encode_frame_v2_from(255, 0x123456, &payload, 92, 17, 42, &mut frame),
        frame.len()
    );
    assert_eq!(
        &frame[..10],
        &[MAGIC_V2, 255, 0, 0, 255, 17, 42, 0x56, 0x34, 0x12]
    );
    assert_eq!(&frame[10..265], &payload);
    assert_eq!(
        u16::from_le_bytes([frame[265], frame[266]]),
        compute_crc(&frame[1..265], 92)
    );
}

#[test]
fn invalid_lengths_leave_the_output_untouched() {
    let mut short = [0xa5; 20];
    assert_eq!(
        encode_frame_v2_from(4, 0, &[0; 9], 50, 255, 190, &mut short),
        0
    );
    assert_eq!(short, [0xa5; 20]);
    let mut long = [0xa5; 268];
    assert_eq!(
        encode_frame_v2_from(4, 0, &[0; 256], 50, 255, 190, &mut long),
        0
    );
    assert_eq!(long, [0xa5; 268]);
}
