//! Control-feel capability updates after an adapter activation.

use super::fixtures::{capabilities, hello, motion_control_frame};
use super::*;

fn descriptor(profile_id: &str, digest: u8) -> ControlFeelDescriptor {
    ControlFeelDescriptor {
        profile_id: profile_id.to_owned(),
        mode: ControlFeelMode::Balanced,
        schema_version: 1,
        profile_sha256: [digest; 32],
        device_profile_sha256: [0x22; 32],
        flight_controller_sha256: [0x33; 32],
    }
}

fn actor_with_control_feel(active: ControlFeelDescriptor) -> EngineActor<RecordingAdapter> {
    let adapter = RecordingAdapter {
        control_feel: Some(active.clone()),
        ..RecordingAdapter::default()
    };
    let mut initial = capabilities();
    initial.control_feel = Some(active);
    let engine = SessionEngine::new(
        initial,
        StalenessPolicy::new(std::time::Duration::from_millis(250)),
        SessionConfig::new(1, "host-test").with_legacy_compatibility(true),
    );
    EngineActor::new(engine, adapter, Instant::now())
}

fn welcomed_profile_id(actor: &mut EngineActor<RecordingAdapter>, client: u64) -> String {
    let outcome = actor.engine.handle_client_message(
        ClientKey::new(client),
        hello(),
        MonoTimestamp::from_nanos(0),
    );
    outcome
        .actions
        .iter()
        .find_map(|action| match action {
            SessionAction::SendToClient {
                envelope: OutboundMessage::Welcome(welcome),
                ..
            } => welcome
                .host_capabilities
                .control_feel
                .as_ref()
                .map(|feel| feel.profile_id.clone()),
            _ => None,
        })
        .expect("welcome control-feel identity")
}

#[test]
fn a_successful_adapter_activation_updates_a_new_welcome() {
    let mut actor = actor_with_control_feel(descriptor("feel-before", 0x10));
    actor.adapter.next_control_feel = Some(descriptor("feel-after", 0x20));

    actor.apply_to_adapter(ClientKey::new(9), motion_control_frame());

    assert_eq!(welcomed_profile_id(&mut actor, 2), "feel-after");
}

#[test]
fn a_rejected_activation_keeps_the_prior_welcome_identity() {
    let mut actor = actor_with_control_feel(descriptor("feel-before", 0x10));
    actor.adapter.next_control_feel = Some(descriptor("feel-after", 0x20));
    actor.adapter.reject_control = Some(RejectReason::Other("activation failed".to_owned()));

    actor.apply_to_adapter(ClientKey::new(9), motion_control_frame());

    assert_eq!(welcomed_profile_id(&mut actor, 2), "feel-before");
}
