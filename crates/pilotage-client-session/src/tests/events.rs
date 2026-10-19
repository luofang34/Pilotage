use super::*;

#[test]
fn telemetry_and_rejections_arrive_as_typed_module_events() {
    let mut engine = engine();
    admit(&mut engine, 7, 42);

    let telemetry = wire::Envelope {
        schema_version: 1,
        payload: Some(wire::envelope::Payload::TelemetrySample(
            wire::TelemetrySample::default(),
        )),
    };
    let actions = engine.handle(
        TransportEvent::DatagramReceived(telemetry.encode_to_vec()),
        0,
    );
    assert!(matches!(
        actions[0],
        ClientAction::Emit(ModuleEvent::Telemetry(_))
    ));

    let rejected = wire::Envelope {
        schema_version: 1,
        payload: Some(wire::envelope::Payload::FrameRejected(
            wire::FrameRejected::default(),
        )),
    };
    let actions = engine.handle(
        TransportEvent::DatagramReceived(rejected.encode_to_vec()),
        0,
    );
    assert!(matches!(
        actions[0],
        ClientAction::Emit(ModuleEvent::ControlRejected(_))
    ));
}
