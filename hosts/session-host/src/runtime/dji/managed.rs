use std::path::PathBuf;
use std::time::Instant;

use pilotage_adapter_api::{
    AdapterCapabilities, ExecutionMode, FlightProcedureSample, VehicleAdapter, VehicleDescriptor,
};
use pilotage_adapter_dji::{DjiAdapter, DjiAttachment, DjiConfig, EncodedVideoStream};
use pilotage_protocol::{VehicleClass, VehicleId};
use tokio::sync::mpsc;

use super::{AttachmentSelection, DjiHostConfig, DjiHostError};

mod delegate;
#[cfg(target_os = "macos")]
mod selection;
#[cfg(target_os = "macos")]
use selection::Selection;

/// The host owns this value independently of all client connections.
pub(in crate::runtime) struct ManagedDji {
    vehicle: VehicleId,
    origin: Instant,
    config: DjiHostConfig,
    virtual_stick: bool,
    adapter: Option<DjiAdapter>,
    identity: Option<u64>,
    #[cfg(target_os = "macos")]
    selection: Selection,
    blocked: bool,
    retained: Option<FlightProcedureSample>,
    streams: mpsc::Sender<Option<EncodedVideoStream>>,
    #[cfg(target_os = "macos")]
    events: Option<pilotage_dji_usb::UsbEvents>,
}

impl ManagedDji {
    pub(super) fn start_blocking(
        vehicle: VehicleId,
        config: DjiHostConfig,
        virtual_stick: bool,
        streams: mpsc::Sender<Option<EncodedVideoStream>>,
    ) -> Result<Self, DjiHostError> {
        #[cfg(target_os = "macos")]
        let events = if matches!(config.attachment, AttachmentSelection::AutoUsb) {
            Some(
                pilotage_dji_usb::UsbEvents::subscribe_blocking()
                    .map_err(DjiHostError::UsbDiscovery)?,
            )
        } else {
            None
        };
        let mut managed = Self {
            vehicle,
            config,
            virtual_stick,
            streams,
            origin: Instant::now(),
            adapter: None,
            identity: None,
            #[cfg(target_os = "macos")]
            selection: Selection::default(),
            blocked: false,
            retained: None,
            #[cfg(target_os = "macos")]
            events,
        };
        match &managed.config.attachment {
            AttachmentSelection::Explicit(attachment) => {
                let adapter = managed
                    .open_blocking(attachment.clone(), managed.config.capture_path.clone())?;
                managed.install(adapter)?;
            }
            #[cfg(target_os = "macos")]
            AttachmentSelection::AutoUsb => managed.discover_blocking(),
        }
        Ok(managed)
    }

    fn open_blocking(
        &self,
        attachment: DjiAttachment,
        capture_path: PathBuf,
    ) -> Result<DjiAdapter, DjiHostError> {
        DjiAdapter::start_blocking(
            self.vehicle,
            DjiConfig {
                attachment,
                capture_path,
                control_mode: self.config.control_mode,
                experimental_flight: self.config.experimental_flight,
                experimental_virtual_stick: self.virtual_stick,
                experimental_virtual_stick_diagnostic: false,
            },
        )
        .map_err(DjiHostError::Adapter)
    }

    fn install(&mut self, mut adapter: DjiAdapter) -> Result<(), DjiHostError> {
        if let Some(stream) = adapter.subscribe_encoded_video() {
            self.streams
                .try_send(Some(stream))
                .map_err(|_| DjiHostError::VideoLifecycle)?;
        }
        self.adapter = Some(adapter);
        tracing::info!(identity = ?self.identity, "DJI transport open; fresh aircraft evidence and control authority required");
        Ok(())
    }

    fn detach_blocking(&mut self) {
        let Some(mut adapter) = self.adapter.take() else {
            return;
        };
        adapter.disconnect_blocking();
        if let Some(procedure) = adapter
            .sample_telemetry()
            .samples
            .into_iter()
            .find_map(|s| s.flight_procedure)
        {
            self.retained = Some(procedure);
        }
        self.blocked |= !adapter.transport_released();
        self.identity = None;
        if self.streams.try_send(None).is_err() {
            self.blocked = true;
            tracing::error!("DJI video lifecycle queue unavailable; automatic reopen fenced");
        }
    }

    fn offline_capabilities(&self) -> AdapterCapabilities {
        AdapterCapabilities {
            execution: ExecutionMode {
                real_time: true,
                physically_embodied: true,
                ..ExecutionMode::default()
            },
            vehicles: vec![VehicleDescriptor {
                id: self.vehicle,
                class: VehicleClass::Multicopter,
                heading_reference: None,
                display_name: "DJI · waiting for USB".into(),
                scopes: Vec::new(),
                link_loss_actions: Vec::new(),
            }],
            adapter_version: env!("CARGO_PKG_VERSION").into(),
            control_feel: None,
        }
    }

    #[cfg(target_os = "macos")]
    fn discover_blocking(&mut self) {
        let candidates = match pilotage_dji_usb::discover_upper_blocking() {
            Ok(candidates) => candidates,
            Err(error) => {
                tracing::warn!(%error, "DJI registry snapshot unavailable; waiting for a new device event");
                return;
            }
        };
        let ids: Vec<_> = candidates.iter().map(|c| c.registry_id).collect();
        let selected = self.selection.observe(&ids);
        if self.identity.is_some_and(|id| !ids.contains(&id)) {
            self.detach_blocking();
            // Expose the missing scopes for one engine step before a new owner.
            self.selection.defer(selected);
            return;
        }
        if self.adapter.is_none() && !self.blocked {
            self.attach_selected_blocking(selected);
        }
    }

    #[cfg(target_os = "macos")]
    fn attach_selected_blocking(&mut self, selected: Option<u64>) {
        let Some(registry_id) = selected else {
            return;
        };
        let mut path = self.config.capture_path.as_os_str().to_os_string();
        path.push(format!(".usb-{registry_id:016x}.jsonl"));
        self.identity = Some(registry_id);
        match self.open_blocking(DjiAttachment::MacUsb { registry_id }, PathBuf::from(path)) {
            Ok(adapter) => {
                if let Err(error) = self.install(adapter) {
                    self.blocked = true;
                    tracing::error!(%error, "DJI automatic attachment fenced");
                }
            }
            Err(error) => {
                self.blocked = matches!(
                    &error,
                    DjiHostError::Adapter(pilotage_adapter_dji::DjiAdapterError::Usb(
                        pilotage_dji_usb::UsbError::StartupUnknown
                            | pilotage_dji_usb::UsbError::CleanupUnknown
                            | pilotage_dji_usb::UsbError::WorkerEnded
                    ))
                );
                self.identity = None;
                tracing::warn!(%error, "DJI open failed; this attachment will not be retried");
            }
        }
    }

    fn update_blocking(&mut self) {
        if self
            .adapter
            .as_ref()
            .is_some_and(|a| a.status_cached().fault.is_some())
        {
            self.detach_blocking();
            return;
        }
        #[cfg(target_os = "macos")]
        if let Some(events) = &self.events {
            match events.changed() {
                Ok(true) => self.discover_blocking(),
                Ok(false) => {
                    if self.adapter.is_none() && !self.blocked {
                        let selected = self.selection.take_deferred();
                        self.attach_selected_blocking(selected);
                    }
                }
                Err(error) => {
                    tracing::error!(%error, "DJI registry observer stopped; recovery fenced");
                    self.blocked = true;
                    self.events = None;
                }
            }
        }
    }
}
