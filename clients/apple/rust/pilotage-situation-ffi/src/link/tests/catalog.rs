use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Duration;

use pilotage_control_web::{ArmOrder, AuthorityScope, GIMBAL_SCOPE, MOTION_SCOPE};

use super::*;

struct CatalogObserver(Sender<LinkEvent>);

impl LinkObserver for CatalogObserver {
    fn on_event(&self, event: LinkEvent) {
        self.0.send(event).ok();
    }
    fn on_state_frame(&self, _frame: Vec<u8>, _accepted_at_ms: u64) {}
    fn on_video_frame(&self, _source_id: u8, _codec: String, _payload: Vec<u8>) {}
}

fn record_events(link: &mut Link) -> Receiver<LinkEvent> {
    let (send, receive) = channel();
    link.delivery = DeliveryQueue::start(Arc::new(CatalogObserver(send)));
    receive
}

fn capabilities() -> wire::HostCapabilities {
    let Some(wire::envelope::Payload::ServerWelcome(welcome)) = two_scope_welcome().payload else {
        panic!("welcome fixture");
    };
    welcome.host_capabilities.expect("catalog fixture")
}

fn refresh(link: &mut Link, capabilities: wire::HostCapabilities) {
    deliver(
        link,
        wire::Envelope {
            schema_version: 1,
            payload: Some(wire::envelope::Payload::HostCapabilities(capabilities)),
        },
    );
}

fn deliver(link: &mut Link, envelope: wire::Envelope) {
    let actions = link.engine.handle(
        TransportEvent::BootstrapReceived(pilotage_protocol::encode_envelope_length_delimited(
            &envelope,
        )),
        0,
    );
    for action in actions {
        let ClientAction::Emit(event) = action else {
            panic!("catalog changes must not send commands");
        };
        link.emit(event);
    }
}

fn next_event(receive: &Receiver<LinkEvent>) -> LinkEvent {
    receive
        .recv_timeout(Duration::from_secs(5))
        .expect("the driver delivers the catalog event")
}

#[test]
fn catalog_refresh_preserves_session_control_and_telegraph_state() {
    let mut link = admitted_link_with_two_lanes();
    let receive = record_events(&mut link);
    assert!(link.telegraph.set_order(ArmOrder::Armed).is_some());
    let motion = link.control.authority_state(AuthorityScope::Motion);
    let gimbal = link.control.authority_state(AuthorityScope::Gimbal);
    let mut catalog = capabilities();
    catalog.vehicles[0].display_name = "updated vehicle".into();
    refresh(&mut link, catalog);

    let LinkEvent::CapabilitiesChanged { catalog } = next_event(&receive) else {
        panic!("a catalog replacement is not a new admission");
    };
    assert_eq!(catalog.session_id, 7);
    assert_eq!(catalog.principal_id, 42);
    assert_eq!(catalog.vehicles[0].display_name, "updated vehicle");
    assert_eq!(catalog.vehicles[0].scopes.len(), 2);
    assert_eq!(link.control.authority_state(AuthorityScope::Motion), motion);
    assert_eq!(link.control.authority_state(AuthorityScope::Gimbal), gimbal);
    assert_eq!(link.telegraph.order(), ArmOrder::Armed);
    assert!(link.engine.holds(1, MOTION_SCOPE));
    assert!(link.engine.holds(1, GIMBAL_SCOPE));
}

#[test]
fn changed_scope_revokes_only_its_runtime_and_shell_grant() {
    let mut link = admitted_link_with_two_lanes();
    let receive = record_events(&mut link);
    let mut catalog = capabilities();
    catalog.vehicles[0].scopes[0].intents[0].max_linear = 1.0;
    refresh(&mut link, catalog);

    assert!(matches!(
        next_event(&receive),
        LinkEvent::LeaseChanged { held: false, scope, .. } if scope == MOTION_SCOPE
    ));
    let LinkEvent::CapabilitiesChanged { catalog } = next_event(&receive) else {
        panic!("the shell receives the revised controls");
    };
    assert_eq!(catalog.vehicles[0].scopes[0].intents[0].max_linear, 1.0);
    assert!(!link.engine.holds(1, MOTION_SCOPE));
    assert!(
        !link
            .control
            .authority_state(AuthorityScope::Motion)
            .granted()
    );
    assert!(link.engine.holds(1, GIMBAL_SCOPE));
    assert!(
        link.control
            .authority_state(AuthorityScope::Gimbal)
            .granted()
    );
    assert!(
        link.motion_actions(MotionDemand {
            roll: 0.0,
            pitch: 0.5,
            throttle: 0.0,
            yaw: 0.0,
        })
        .is_empty()
    );
}

#[test]
fn catalog_replacement_clears_only_invalidated_pending_requests_and_ignores_late_grants() {
    let mut link = admitted_link();
    let receive = record_events(&mut link);
    assert!(!link.select_video_source_actions(2).is_empty());
    refresh(&mut link, capabilities());
    assert!(matches!(
        next_event(&receive),
        LinkEvent::CapabilitiesChanged { .. }
    ));
    assert!(link.pending_gimbal_engage);
    assert!(link.engine.lease_pending(1, GIMBAL_SCOPE));

    let mut changed = capabilities();
    changed.vehicles[0].scopes[1].intents[0].max_angular = 0.3;
    refresh(&mut link, changed.clone());
    assert!(matches!(
        next_event(&receive),
        LinkEvent::CapabilitiesChanged { .. }
    ));
    assert!(!link.pending_gimbal_engage);
    assert!(!link.engine.lease_pending(1, GIMBAL_SCOPE));

    deliver(
        &mut link,
        wire::Envelope {
            schema_version: 1,
            payload: Some(wire::envelope::Payload::LeaseResponse(lease_response(
                GIMBAL_SCOPE,
                true,
                9,
            ))),
        },
    );
    refresh(&mut link, changed);
    assert!(matches!(
        next_event(&receive),
        LinkEvent::CapabilitiesChanged { .. }
    ));
    assert!(
        !link
            .control
            .authority_state(AuthorityScope::Gimbal)
            .granted()
    );
    assert_eq!(link.engine.control_generation(1, GIMBAL_SCOPE), None);

    link.started = Instant::now() - Duration::from_secs(6);
    assert!(!link.gimbal_keepalive_actions().is_empty());
    assert!(link.engine.lease_pending(1, GIMBAL_SCOPE));
}

#[test]
fn unrequested_grant_generation_cannot_replace_a_valid_runtime_grant() {
    let mut link = admitted_link_with_two_lanes();
    let receive = record_events(&mut link);
    let motion = link.control.authority_state(AuthorityScope::Motion);
    deliver(
        &mut link,
        wire::Envelope {
            schema_version: 1,
            payload: Some(wire::envelope::Payload::LeaseResponse(lease_response(
                MOTION_SCOPE,
                true,
                100,
            ))),
        },
    );
    refresh(&mut link, capabilities());
    assert!(matches!(
        next_event(&receive),
        LinkEvent::CapabilitiesChanged { .. }
    ));
    assert_eq!(link.control.authority_state(AuthorityScope::Motion), motion);
    assert_eq!(link.engine.control_generation(1, MOTION_SCOPE), Some(4));
}
