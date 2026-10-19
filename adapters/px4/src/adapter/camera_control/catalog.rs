use pilotage_adapter_api::{ActionCapability, ScopeDescriptor, payload_capabilities_to_wire};
use pilotage_mavlink::CameraReport;
use pilotage_protocol::{ActionKind, ScopeId, wire};
use turret_core::{Capability, PayloadCapabilities, UnavailableReason};

pub(super) fn camera_scope((component, device): (u8, u8)) -> String {
    format!("vehicle.camera.{component}.{device}")
}

pub(super) fn camera_key(scope: &str) -> Option<(u8, u8)> {
    let (component, device) = scope.strip_prefix("vehicle.camera.")?.split_once('.')?;
    Some((component.parse().ok()?, device.parse().ok()?))
}

pub(super) fn core_capabilities(
    report: &CameraReport,
) -> Result<PayloadCapabilities, turret_core::Error> {
    let basic_zoom = report.flags.is_some_and(|flags| flags & 64 != 0);
    let mut capabilities =
        turret_mavlink::camera_capabilities(report.capability_revision, basic_zoom)?;
    if report.flags.is_some() && !basic_zoom {
        capabilities.zoom = Capability::Unsupported;
    }
    if let Capability::Available(mut zoom) = capabilities.zoom {
        if report.travel_fraction.is_some() {
            zoom.feedback.travel_fraction = zoom.absolute.travel_fraction;
        }
        capabilities.zoom = if report.available {
            Capability::Available(zoom)
        } else {
            Capability::Unavailable {
                capability: zoom,
                reason: UnavailableReason::Disconnected,
            }
        };
    }
    Ok(capabilities)
}

pub(super) fn descriptor(
    key: (u8, u8),
    report: &CameraReport,
) -> Result<ScopeDescriptor, turret_core::Error> {
    let scope = camera_scope(key);
    let capabilities = core_capabilities(report)?;
    let mut payload = payload_capabilities_to_wire(&scope, &capabilities, &[&scope]);
    payload.connection_epoch = report.epoch;
    payload.connection_role = wire::PayloadConnectionRole::DirectDevice as i32;
    Ok(ScopeDescriptor {
        scope: ScopeId::new(scope),
        authority_group: None,
        axes: vec![],
        intents: vec![],
        legacy: None,
        payload: Some(payload),
        actions: if matches!(
            capabilities.zoom,
            Capability::Available(_) | Capability::Unavailable { .. }
        ) {
            vec![ActionCapability {
                action: ActionKind::CameraZoom,
                mode_targets: vec![],
                feel_targets: vec![],
            }]
        } else {
            vec![]
        },
    })
}
