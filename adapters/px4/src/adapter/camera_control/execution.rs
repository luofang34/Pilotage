use super::catalog::{camera_key, core_capabilities};
use pilotage_mavlink::{
    CameraReport, CameraWriteRequirement, LinkState, OutboundCommand, PayloadChannel,
    PayloadTiming, SendFence,
};
use pilotage_protocol::{ScopedControlFrame, ZoomCommand, ZoomUnit, wire};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tracing::warn;
use turret_core::{Effect, PayloadCapabilities, PayloadCommand};

#[derive(Debug, thiserror::Error)]
pub(super) enum CameraCommandError {
    #[error("camera information unavailable")]
    Missing,
    #[error("camera command unsupported: {0}")]
    Contract(#[from] turret_core::Error),
    #[error("camera execution refused: {0}")]
    Execution(#[from] pilotage_mavlink::PayloadExecutionError),
    #[error("camera command queue unavailable: {0}")]
    Queue(#[from] tokio::sync::mpsc::error::TrySendError<()>),
}

#[derive(Debug)]
struct Endpoint {
    requirement: CameraWriteRequirement,
    capabilities: PayloadCapabilities,
    channel: PayloadChannel,
}

#[derive(Debug)]
pub(in crate::adapter) struct CameraControl {
    commands: tokio::sync::mpsc::Sender<OutboundCommand>,
    system_id: u8,
    next_probe: Instant,
    endpoints: BTreeMap<(u8, u8), Endpoint>,
}

impl CameraControl {
    pub(in crate::adapter) fn new(
        commands: tokio::sync::mpsc::Sender<OutboundCommand>,
        system_id: u8,
    ) -> Self {
        Self {
            commands,
            system_id,
            next_probe: Instant::now(),
            endpoints: BTreeMap::new(),
        }
    }

    pub(in crate::adapter) fn probe(&mut self, now: Instant) {
        if now < self.next_probe {
            return;
        }
        self.next_probe = now + Duration::from_secs(2);
        for message in [259.0, 260.0] {
            self.commands
                .try_send(OutboundCommand {
                    payload_write: None,
                    receipt: None,
                    fence: None,
                    command: 512,
                    params: [message, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
                    target_system: self.system_id,
                    target_component: 0,
                    send_before: now.checked_add(Duration::from_millis(250)),
                })
                .ok();
        }
    }

    pub(super) fn synchronize(
        &mut self,
        reports: &[(CameraWriteRequirement, CameraReport)],
        started: Instant,
    ) {
        self.endpoints.retain(|key, endpoint| {
            if reports.iter().any(|(requirement, _)| {
                requirement.key == *key && same_connection(*requirement, endpoint.requirement)
            }) {
                return true;
            }
            revoke(&endpoint.channel);
            false
        });
        for (requirement, report) in reports {
            let key = requirement.key;
            let capabilities = match core_capabilities(report) {
                Ok(value) => value,
                Err(error) => {
                    warn!(%error, ?key, "camera capabilities invalid");
                    continue;
                }
            };
            let endpoint = self.endpoints.entry(key).or_insert_with(|| Endpoint {
                requirement: *requirement,
                capabilities,
                channel: PayloadChannel::new(capabilities, report.epoch, started),
            });
            if endpoint.capabilities != capabilities {
                if let Err(error) = endpoint.channel.update_capabilities(capabilities) {
                    warn!(%error, ?key, "camera capability update failed");
                    continue;
                }
                endpoint.capabilities = capabilities;
            }
            endpoint.requirement = *requirement;
            match endpoint.channel.poll_effect() {
                Ok(Effect::None) => {}
                Ok(effect) => warn!(
                    ?effect,
                    ?key,
                    "camera command ended; physical cancellation is unconfirmed"
                ),
                Err(error) => warn!(%error, ?key, "camera execution polling failed"),
            }
        }
    }

    pub(super) fn send(
        &self,
        key: (u8, u8),
        command: ZoomCommand,
        id: u64,
        timing: PayloadTiming,
        state: Arc<Mutex<LinkState>>,
    ) -> Result<(), CameraCommandError> {
        let endpoint = self
            .endpoints
            .get(&key)
            .ok_or(CameraCommandError::Missing)?;
        let payload = PayloadCommand::Zoom(zoom_command(command)?);
        let camera_id = if key.1 == 0 { key.0 } else { key.1 };
        let turret_mavlink::Demand::CommandLong { command, params } =
            turret_mavlink::encode_payload(payload, 0, camera_id)?
        else {
            return Err(turret_core::Error::Unsupported.into());
        };
        let permit = self.commands.try_reserve()?;
        let write = endpoint.channel.submit(payload, id, timing)?;
        permit.send(OutboundCommand {
            payload_write: Some(write),
            receipt: None,
            fence: Some(SendFence::default().for_camera(state, endpoint.requirement)),
            command,
            params,
            target_system: self.system_id,
            target_component: key.0,
            send_before: Some(timing.send_before),
        });
        Ok(())
    }

    pub(in crate::adapter) fn invalidate(&mut self, scope: &str) {
        if let Some(endpoint) = camera_key(scope).and_then(|key| self.endpoints.get(&key)) {
            revoke(&endpoint.channel);
        }
    }

    pub(super) fn progress_cached(
        &self,
        requirement: CameraWriteRequirement,
    ) -> Option<wire::PayloadCommandProgress> {
        let endpoint = self.endpoints.get(&requirement.key)?;
        if !same_connection(requirement, endpoint.requirement) {
            return None;
        }
        let snapshot = endpoint.channel.snapshot_cached().ok()??;
        Some(pilotage_adapter_api::payload_progress_to_wire(
            &snapshot.progress,
            Some(&snapshot.command),
            snapshot.updated_at,
        ))
    }
}

fn same_connection(a: CameraWriteRequirement, b: CameraWriteRequirement) -> bool {
    a.key == b.key
        && a.epoch == b.epoch
        && a.source_id == b.source_id
        && a.source_incarnation == b.source_incarnation
        && a.source_epoch == b.source_epoch
}

fn revoke(channel: &PayloadChannel) {
    if let Err(error) = channel.revoke() {
        warn!(%error, "camera command revocation failed");
    }
}

pub(super) fn command_timing(frame: &ScopedControlFrame, now: Instant) -> Option<PayloadTiming> {
    let admitted = frame.admitted_timing;
    let send = admitted.map_or(Duration::from_millis(250), |timing| timing.send_remaining);
    let completion_before = match admitted.and_then(|timing| timing.completion_timeout) {
        Some(duration) => Some(now.checked_add(duration)?),
        None => None,
    };
    Some(PayloadTiming {
        send_before: now.checked_add(send)?,
        valid_until: None,
        ack_before: None,
        completion_before,
    })
}

fn zoom_command(command: ZoomCommand) -> Result<turret_core::ZoomCommand, turret_core::Error> {
    Ok(match command {
        ZoomCommand::Step(steps) => turret_core::ZoomCommand::Step(steps),
        ZoomCommand::Continuous(rate) => turret_core::ZoomCommand::Continuous(
            turret_core::NormalizedZoomRate::new(rate as f32 / 1_000_000.0)?,
        ),
        ZoomCommand::Absolute { unit, value_micros } => {
            let unit = match unit {
                ZoomUnit::Ratio => turret_core::ZoomUnit::Ratio,
                ZoomUnit::TravelFraction => turret_core::ZoomUnit::TravelFraction,
                ZoomUnit::FocalLengthMm => turret_core::ZoomUnit::FocalLengthMm,
                ZoomUnit::HorizontalFovRadians => turret_core::ZoomUnit::HorizontalFovRadians,
            };
            turret_core::ZoomCommand::Absolute(turret_core::ZoomAbsolute::new(
                unit,
                value_micros as f32 / 1_000_000.0,
            )?)
        }
    })
}
