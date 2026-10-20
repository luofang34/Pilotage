use super::*;
use turret_core::{AngularRate, Axes, GimbalRate, ReferenceFrame};

fn rate(pitch: Option<f32>, yaw: Option<f32>) -> PayloadCommand {
    PayloadCommand::GimbalRate(GimbalRate {
        rates: Axes {
            pitch: pitch.map(|value| AngularRate::radians_per_second(value).unwrap()),
            roll: None,
            yaw: yaw.map(|value| AngularRate::radians_per_second(value).unwrap()),
        },
        frame: ReferenceFrame::HorizonVehicleYaw,
    })
}

fn streaming() -> PayloadTiming {
    let now = Instant::now();
    PayloadTiming {
        send_before: now + Duration::from_secs(1),
        valid_until: Some(now + Duration::from_secs(10)),
        ack_before: None,
        completion_before: None,
    }
}

#[test]
fn pending_recenter_does_not_replace_actual_active_stop_responsibility() {
    let channel = channel();
    let pitch = rate(Some(0.4), None);
    let active = channel.submit(pitch, 1, streaming()).unwrap();
    assert!(active.write(1, || Ok(1)).unwrap());
    let pending = channel
        .submit(PayloadCommand::Recenter, 2, timing())
        .unwrap();
    channel.revoke().unwrap();
    let effect = channel.poll_effect().unwrap().unwrap();
    assert_eq!(effect.command.payload, pitch);
    assert_eq!(effect.command.identity.id, 1);
    assert_eq!(effect.effect, Effect::StopContinuous);
    assert_eq!(
        channel
            .snapshot_cached()
            .unwrap()
            .unwrap()
            .command
            .identity
            .id,
        2
    );
    assert!(
        !pending
            .write(1, || panic!("revoked pending write"))
            .unwrap()
    );
    assert_eq!(channel.poll_effect().unwrap(), Some(effect));
    assert!(matches!(
        channel.submit(pitch, 3, streaming()),
        Err(PayloadExecutionError::UnresolvedEffect)
    ));
    channel.revoke().unwrap();
    assert_eq!(channel.poll_effect().unwrap(), Some(effect));
    channel.resolve_effect(effect.command.identity).unwrap();
    assert_eq!(channel.poll_effect().unwrap(), None);
    assert!(channel.submit(pitch, 4, streaming()).is_ok());
}

#[test]
fn active_expiry_stops_the_sent_axis_while_recenter_waits() {
    let channel = channel();
    let pitch = rate(Some(0.4), None);
    let stream_timing = streaming();
    let active = channel.submit(pitch, 1, stream_timing).unwrap();
    assert!(active.write(1, || Ok(1)).unwrap());
    channel
        .submit(PayloadCommand::Recenter, 2, timing())
        .unwrap();
    let effect = channel
        .poll_effect_at(stream_timing.valid_until.unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(effect.command.payload, pitch);
    assert_eq!(effect.effect, Effect::StopContinuous);
    assert_eq!(effect.command.identity.id, 1);
}

#[test]
fn partial_write_retains_both_commands_until_each_effect_is_resolved() {
    let channel = channel();
    let pitch = rate(Some(0.4), None);
    let replacement = rate(Some(-0.4), None);
    let active = channel.submit(pitch, 1, streaming()).unwrap();
    assert!(active.write(1, || Ok(1)).unwrap());
    let attempted = channel.submit(replacement, 2, streaming()).unwrap();
    assert!(attempted.write(2, || Ok(1)).is_err());
    let first = channel.poll_effect().unwrap().unwrap();
    assert_eq!(first.command.payload, pitch);
    assert_eq!(first.effect, Effect::StopContinuous);
    assert_eq!(channel.poll_effect().unwrap(), Some(first));
    channel.resolve_effect(first.command.identity).unwrap();
    let second = channel.poll_effect().unwrap().unwrap();
    assert_eq!(second.command.payload, replacement);
    assert_eq!(second.effect, Effect::StopContinuous);
    assert!(matches!(
        channel.submit(pitch, 3, streaming()),
        Err(PayloadExecutionError::UnresolvedEffect)
    ));
    channel.resolve_effect(second.command.identity).unwrap();
    assert_eq!(channel.poll_effect().unwrap(), None);
    channel.revoke().unwrap();
    assert_eq!(channel.poll_effect().unwrap(), None);
}

#[test]
fn unimplemented_finite_zoom_cancellation_is_retained_and_blocks_new_motion() {
    let channel = PayloadChannel::new(
        turret_mavlink::camera_capabilities(1, true).unwrap(),
        1,
        Instant::now(),
    );
    let zoom = PayloadCommand::Zoom(turret_core::ZoomCommand::Absolute(
        turret_core::ZoomAbsolute::new(turret_core::ZoomUnit::TravelFraction, 0.5).unwrap(),
    ));
    let active = channel.submit(zoom, 1, timing()).unwrap();
    assert!(active.write(1, || Ok(1)).unwrap());
    channel.revoke().unwrap();
    let effect = channel.poll_effect().unwrap().unwrap();
    assert_eq!(effect.command.payload, zoom);
    assert_eq!(effect.effect, Effect::CancelAction);
    assert_eq!(
        turret_mavlink::stop_command(zoom),
        Err(turret_core::Error::Unsupported)
    );
    assert_eq!(channel.poll_effect().unwrap(), Some(effect));
    assert!(matches!(
        channel.submit(zoom, 2, timing()),
        Err(PayloadExecutionError::UnresolvedEffect)
    ));
}

#[test]
fn active_partial_axes_require_a_resolved_stop_before_mask_or_frame_changes() {
    let channel = channel();
    let pitch = rate(Some(0.4), None);
    let active = channel.submit(pitch, 1, streaming()).unwrap();
    assert!(active.write(1, || Ok(1)).unwrap());
    assert!(matches!(
        channel.submit(rate(None, Some(0.4)), 2, streaming()),
        Err(PayloadExecutionError::AxisReplacement)
    ));
    let PayloadCommand::GimbalRate(mut other_frame) = pitch else {
        panic!("expected rate");
    };
    other_frame.frame = ReferenceFrame::Vehicle;
    assert!(matches!(
        channel.submit(PayloadCommand::GimbalRate(other_frame), 3, streaming()),
        Err(PayloadExecutionError::AxisReplacement)
    ));
    let next = channel
        .submit(rate(Some(-0.4), None), 4, streaming())
        .unwrap();
    assert!(next.write(1, || Ok(1)).unwrap());
    channel.revoke().unwrap();
    let effect = channel.poll_effect().unwrap().unwrap();
    assert_eq!(effect.command.identity.id, 4);
    channel.resolve_effect(effect.command.identity).unwrap();
    let yaw = channel
        .submit(rate(None, Some(0.4)), 5, streaming())
        .unwrap();
    assert!(yaw.write(1, || Ok(1)).unwrap());
}
