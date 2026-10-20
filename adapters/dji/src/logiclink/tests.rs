use super::*;

const MOBILE: &[u8] = include_bytes!("fixtures/mini3-upper-receive.bin");

#[test]
fn real_usb_capture_preserves_control_video_and_padding_across_every_chunk_size() {
    for size in [1, 7, 8, 9, 512, 4096, MOBILE.len()] {
        let mut parser = LogicLinkParser::new();
        let mut controls = turret_dji::DumlParser::new();
        let mut frames = Vec::new();
        let mut video = Vec::new();
        for bytes in MOBILE.chunks(size) {
            parser
                .push(bytes, |frame| match frame.channel {
                    CONTROL_CHANNEL => controls.push(frame.payload, |frame| frames.push(frame)),
                    VIDEO_CHANNEL => video.extend_from_slice(frame.payload),
                    other => panic!("unexpected fixture channel {other}"),
                })
                .unwrap();
        }
        parser.finish().unwrap();
        assert_eq!(parser.stats().envelopes, 41);
        assert!(parser.stats().padding_bytes > 0);
        assert_eq!(frames.len(), 39);
        assert_eq!(controls.stats().discarded_bytes, 0);
        assert_eq!(controls.buffered_bytes(), 0);
        assert_eq!(video.len(), 4080);
        assert_eq!(
            frames
                .iter()
                .filter(|frame| (frame.command_set(), frame.command_id()) == (4, 5))
                .count(),
            3
        );
    }
}

#[test]
fn unknown_empty_channel_and_zero_payload_bytes_are_not_padding() {
    let mut parser = LogicLinkParser::new();
    let bytes = [
        0x55, 0xcc, 1, 0, 0, 0, 0, 0, 0, 0, 0x55, 0xcc, 2, 0, 3, 0, 0, 0, 0, 0, 0,
    ];
    let mut frames = Vec::new();
    parser
        .push(&bytes, |frame| {
            frames.push((frame.channel, frame.payload.to_vec()))
        })
        .unwrap();
    assert_eq!(frames, [(1, vec![]), (2, vec![0, 0, 0])]);
    assert_eq!(parser.stats().padding_bytes, 2);
    parser.finish().unwrap();
}

#[test]
fn corrupt_headers_latch_instead_of_scanning_into_payloads() {
    for (header, expected) in [
        ([0x55, 0xcd, 0, 0, 0, 0, 0, 0], LogicLinkError::Signature),
        (
            [0x55, 0xcc, 0x49, 0x57, 0, 0, 1, 0],
            LogicLinkError::PayloadLength(65536),
        ),
    ] {
        let mut parser = LogicLinkParser::new();
        assert_eq!(parser.push(&header, |_| panic!()), Err(expected));
        assert_eq!(
            parser.push(MOBILE, |_| panic!()),
            Err(LogicLinkError::Invalidated)
        );
    }
}

#[test]
fn eof_is_distinct_from_an_ordinary_usb_transfer_boundary() {
    let mut parser = LogicLinkParser::new();
    parser.push(&MOBILE[..7], |_| panic!()).unwrap();
    assert_eq!(parser.buffered_bytes(), 7);
    assert_eq!(parser.finish(), Err(LogicLinkError::Truncated(7)));
    assert_eq!(
        parser.push(&MOBILE[7..], |_| panic!()),
        Err(LogicLinkError::Invalidated)
    );
}

#[test]
fn encoding_a_validated_command_preserves_its_checksums_and_route() {
    let mut duml = turret_dji::DumlParser::new();
    let mut commands = Vec::new();
    duml.push(turret_dji::rc_version_request().as_slice(), |frame| {
        commands.push(frame)
    });
    let command = &commands[0];
    let bytes = encode_control(command);
    let mut mux = LogicLinkParser::new();
    let mut count = 0;
    mux.push(&bytes, |frame| {
        assert_eq!(frame.channel, CONTROL_CHANNEL);
        assert_eq!(frame.payload, command.raw());
        count += 1;
    })
    .unwrap();
    assert_eq!(count, 1);
}
