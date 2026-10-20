use super::*;

fn envelope(channel: u16, payload: &[u8]) -> Vec<u8> {
    let mut result = vec![0x55, 0xcc];
    result.extend_from_slice(&channel.to_le_bytes());
    result.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    result.extend_from_slice(payload);
    result
}

#[test]
fn video_and_unknown_channels_never_reach_the_command_parser() {
    let mut input = Input::new(Framing::LogicLink);
    let command = turret_dji::rc_version_request();
    for channel in [VIDEO_CHANNEL, 0x1234] {
        assert!(
            input
                .receive(&envelope(channel, &command), Duration::from_secs(1))
                .unwrap()
                .is_empty()
        );
    }
    let chunk = input.take_video().unwrap();
    assert_eq!(chunk.bytes, command);
    assert_eq!(chunk.received_at, Duration::from_secs(1));
    assert_eq!(chunk.continuity, 0);
    assert_eq!(input.unknown_envelopes(), 1);
    assert!(input.command_boundary());
}

#[test]
fn partial_duml_can_cross_envelopes_while_video_has_its_own_boundaries() {
    let mut input = Input::new(Framing::LogicLink);
    let command = turret_dji::rc_version_request();
    let first = envelope(CONTROL_CHANNEL, &command[..5]);
    assert!(input.receive(&first, Duration::ZERO).unwrap().is_empty());
    assert!(!input.command_boundary());
    input
        .receive(&envelope(VIDEO_CHANNEL, &[1, 2, 3]), Duration::ZERO)
        .unwrap();
    let frames = input
        .receive(&envelope(CONTROL_CHANNEL, &command[5..]), Duration::ZERO)
        .unwrap();
    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0].raw(), command);
    assert!(input.command_boundary());
}

#[test]
fn bounded_video_queue_marks_loss_without_disabling_telemetry() {
    let mut input = Input::new(Framing::LogicLink);
    let video = envelope(VIDEO_CHANNEL, &[0; 16 * 1024]);
    for _ in 0..=VIDEO_BUDGET_CHUNKS {
        input.receive(&video, Duration::ZERO).unwrap();
    }
    assert_eq!(input.dropped_video_bytes(), 16 * 1024);
    for _ in 0..VIDEO_BUDGET_CHUNKS {
        assert_eq!(input.take_video().unwrap().continuity, 0);
    }
    input.receive(&video, Duration::ZERO).unwrap();
    let chunk = input.take_video().unwrap();
    assert_eq!(chunk.continuity, 1);
    assert_eq!(chunk.bytes.len(), 16 * 1024);
    assert!(input.take_video().is_none());
    let frames = input
        .receive(
            &envelope(CONTROL_CHANNEL, &turret_dji::rc_version_request()),
            Duration::ZERO,
        )
        .unwrap();
    assert_eq!(frames.len(), 1);
}

#[test]
fn queue_count_is_bounded_for_small_video_segments() {
    let mut input = Input::new(Framing::LogicLink);
    for _ in 0..=VIDEO_BUDGET_CHUNKS {
        input
            .receive(&envelope(VIDEO_CHANNEL, &[1]), Duration::ZERO)
            .unwrap();
    }
    assert_eq!(input.dropped_video_bytes(), 1);
    for _ in 0..VIDEO_BUDGET_CHUNKS {
        assert_eq!(input.take_video().unwrap().continuity, 0);
    }
    input
        .receive(&envelope(VIDEO_CHANNEL, &[1]), Duration::ZERO)
        .unwrap();
    assert_eq!(input.take_video().unwrap().continuity, 1);
    assert!(input.take_video().is_none());
}

#[test]
fn invalid_duml_inside_a_valid_envelope_is_a_control_fault() {
    let mut input = Input::new(Framing::LogicLink);
    let mut command = turret_dji::rc_version_request();
    command[3] ^= 1;
    assert!(matches!(
        input.receive(&envelope(CONTROL_CHANNEL, &command), Duration::ZERO),
        Err(DjiFault::Framing)
    ));
}

#[test]
fn mac_startup_burst_retains_every_video_segment_before_the_consumer_runs() {
    // These envelope lengths preserve a captured Mac startup burst.
    let lengths = [
        2040, 2040, 300, 2040, 2040, 2040, 1756, 2040, 2040, 2040, 560, 2040, 2040, 2040, 2040,
        2040, 2040, 2040, 873, 2040, 2040, 2040, 2040, 2040, 2040, 2040, 2040, 2040, 2040, 849,
        2040, 2040, 2040, 2040, 2040, 2040, 660, 2040, 2040, 2040, 2040, 1443, 2040, 2040, 2040,
        2040,
    ];
    assert_eq!(lengths.iter().sum::<usize>(), 86_001);
    let mut input = Input::new(Framing::LogicLink);
    let mut video = input.subscribe_video().unwrap();
    for (index, length) in lengths.into_iter().enumerate() {
        input
            .receive(
                &envelope(VIDEO_CHANNEL, &vec![index as u8; length]),
                Duration::from_millis(234),
            )
            .unwrap();
    }
    assert_eq!(input.dropped_video_bytes(), 0);
    for (index, length) in lengths.into_iter().enumerate() {
        let chunk = video.try_recv().unwrap();
        assert_eq!(chunk.bytes, vec![index as u8; length]);
        assert_eq!(chunk.continuity, 0);
    }
    assert!(video.try_recv().is_err());
}

#[test]
fn pending_control_and_unidentified_headers_block_movement_but_video_does_not() {
    for channel in [CONTROL_CHANNEL, VIDEO_CHANNEL] {
        let mut input = Input::new(Framing::LogicLink);
        let frame = envelope(channel, &[1; 10]);
        input.receive(&frame[..7], Duration::ZERO).unwrap();
        assert!(!input.command_boundary());
        input.receive(&frame[7..10], Duration::ZERO).unwrap();
        assert_eq!(input.command_boundary(), channel == VIDEO_CHANNEL);
    }
}
