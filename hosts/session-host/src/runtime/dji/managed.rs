use std::path::PathBuf;
use std::time::Instant;

use pilotage_adapter_api::{
    AdapterCapabilities, ExecutionMode, FlightProcedureSample, VehicleAdapter, VehicleDescriptor,
};
use pilotage_adapter_dji::{
    DjiAdapter, DjiAttachment, DjiConfig, DjiTelemetrySource, EncodedVideoStream,
};
use pilotage_protocol::{VehicleClass, VehicleId};
use tokio::sync::mpsc;

use super::{AttachmentSelection, DjiHostConfig, DjiHostError};

mod delegate;
mod operation;
use operation::Operation;
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
    telemetry_source: Option<DjiTelemetrySource>,
    identity: Option<u64>,
    #[cfg(target_os = "macos")]
    selection: Selection,
    blocked: bool,
    pending: Option<Operation<Completion>>,
    candidates: Vec<u64>,
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
        origin: Instant,
    ) -> Result<Self, DjiHostError> {
        #[cfg(target_os = "macos")]
        let events = if matches!(config.attachment, AttachmentSelection::AutoUsb) {
            Some(
                pilotage_dji_usb::UsbEvents::subscribe_access_blocking(config.usb_ground_recovery)
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
            origin,
            adapter: None,
            telemetry_source: None,
            identity: None,
            #[cfg(target_os = "macos")]
            selection: Selection::default(),
            blocked: false,
            pending: None,
            candidates: Vec::new(),
            retained: None,
            #[cfg(target_os = "macos")]
            events,
        };
        match &managed.config.attachment {
            AttachmentSelection::Explicit(attachment) => {
                managed.begin_open(attachment.clone(), managed.config.capture_path.clone())?;
            }
            #[cfg(target_os = "macos")]
            AttachmentSelection::AutoUsb => managed.discover_blocking(),
        }
        Ok(managed)
    }

    fn begin_open(
        &mut self,
        attachment: DjiAttachment,
        capture_path: PathBuf,
    ) -> Result<(), DjiHostError> {
        let vehicle = self.vehicle;
        let origin = self.origin;
        let previous = self.telemetry_source;
        let config = DjiConfig {
            attachment,
            capture_path,
            control_mode: self.config.control_mode,
            experimental_flight: self.config.experimental_flight,
            experimental_virtual_stick: self.virtual_stick,
            experimental_virtual_stick_diagnostic: false,
            heading_reference: self.config.heading_reference,
            gimbal_yaw_frame: self.config.gimbal_yaw_frame,
        };
        self.pending = Some(Operation::start(move || {
            Completion::Opened(
                DjiAdapter::start_with_host_clock_blocking(vehicle, config, origin, previous)
                    .map(Box::new)
                    .map_err(DjiHostError::Adapter),
            )
        })?);
        Ok(())
    }

    fn install(&mut self, mut adapter: DjiAdapter) {
        if let Some(stream) = adapter.subscribe_encoded_video()
            && self.streams.try_send(Some(stream)).is_err()
        {
            self.blocked = true;
            self.begin_close(adapter);
            tracing::error!(
                "DJI video lifecycle unavailable; closing transport off the control loop"
            );
            return;
        }
        self.telemetry_source = Some(adapter.telemetry_source());
        self.adapter = Some(adapter);
        tracing::info!(identity = ?self.identity, "DJI transport open; fresh aircraft evidence and control authority required");
    }

    fn detach(&mut self) {
        let Some(mut adapter) = self.adapter.take() else {
            return;
        };
        adapter.fence_transport();
        if let Some(procedure) = adapter
            .sample_telemetry()
            .samples
            .into_iter()
            .find_map(|s| s.flight_procedure)
        {
            self.retained = Some(procedure);
        }
        self.begin_close(adapter);
        self.identity = None;
        if self.streams.try_send(None).is_err() {
            self.blocked = true;
            tracing::error!("DJI video lifecycle queue unavailable; automatic reopen fenced");
        }
    }

    fn connection_label(&self) -> &'static str {
        #[cfg(target_os = "macos")]
        if let Some(events) = &self.events {
            use pilotage_dji_usb::UsbAccessStatus as State;
            return match events.access_status_cached() {
                State::Waiting => "DJI · waiting for connection",
                State::AuthorizationRequired => "DJI · unlock or authorize this Mac",
                State::Ambiguous => "DJI · select one connection",
                State::Handshake => "DJI · connecting",
                State::DataInterfacePresent => "DJI · opening data interface",
                State::ReconnectRequired => "DJI · handshake timed out; reconnect required",
                State::Blocked => "DJI · USB access unavailable",
            };
        }
        "DJI · waiting for USB"
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
                heading_reference: self.config.heading_reference,
                display_name: self.connection_label().into(),
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
        self.candidates = ids.clone();
        if self.identity.is_some_and(|id| !ids.contains(&id)) {
            self.detach();
            // Expose the missing scopes for one engine step before a new owner.
            self.selection.defer(selected);
            return;
        }
        if self.adapter.is_none() && !self.blocked {
            if self.pending.is_none() {
                self.attach_selected(selected);
            } else {
                self.selection.defer(selected);
            }
        }
    }

    #[cfg(target_os = "macos")]
    fn attach_selected(&mut self, selected: Option<u64>) {
        let Some(registry_id) = selected else {
            return;
        };
        if self.events.as_ref().is_some_and(|events| {
            events.access_status_cached() != pilotage_dji_usb::UsbAccessStatus::DataInterfacePresent
        }) {
            self.selection.defer(Some(registry_id));
            return;
        }
        let mut path = self.config.capture_path.as_os_str().to_os_string();
        path.push(format!(".usb-{registry_id:016x}.jsonl"));
        self.identity = Some(registry_id);
        if let Err(error) =
            self.begin_open(DjiAttachment::MacUsb { registry_id }, PathBuf::from(path))
        {
            self.blocked = true;
            tracing::error!(%error, "DJI connection task could not start");
        }
    }

    fn begin_close(&mut self, mut adapter: DjiAdapter) {
        match Operation::start(move || {
            adapter.disconnect_blocking();
            Completion::Closed(adapter.transport_released())
        }) {
            Ok(operation) => self.pending = Some(operation),
            Err(error) => {
                self.blocked = true;
                tracing::error!(%error, "DJI cleanup task failed");
            }
        }
    }

    fn poll_operation(&mut self) {
        let Some(operation) = &mut self.pending else {
            return;
        };
        let result = operation.poll();
        if matches!(result, Ok(None)) {
            if operation.expired() && !self.blocked {
                self.blocked = true;
                tracing::error!("DJI connection task exceeded deadline; reopening fenced");
            }
            return;
        }
        self.pending = None;
        match result {
            Ok(Some(Completion::Opened(Ok(adapter)))) => {
                if self.blocked
                    || self
                        .identity
                        .is_some_and(|id| !self.candidates.contains(&id))
                {
                    self.begin_close(*adapter);
                } else {
                    self.install(*adapter);
                }
            }
            Ok(Some(Completion::Opened(Err(error)))) => {
                #[cfg(target_os = "macos")]
                {
                    self.blocked |= matches!(
                        &error,
                        DjiHostError::Adapter(pilotage_adapter_dji::DjiAdapterError::Usb(
                            pilotage_dji_usb::UsbError::StartupUnknown
                                | pilotage_dji_usb::UsbError::CleanupUnknown
                                | pilotage_dji_usb::UsbError::WorkerEnded
                                | pilotage_dji_usb::UsbError::Native { stage: 15 | 17, .. }
                        ))
                    );
                }
                self.identity = None;
                tracing::warn!(%error, "DJI open failed; awaiting an eligible state change");
            }
            Ok(Some(Completion::Closed(released))) => self.blocked |= !released,
            Err(error) => {
                self.blocked = true;
                tracing::error!(%error, "DJI lifecycle worker ended");
            }
            Ok(None) => {}
        }
    }

    fn update_blocking(&mut self) {
        self.poll_operation();
        #[cfg(target_os = "macos")]
        if let Some(events) = &self.events {
            events.permit_handshake(
                self.adapter.is_none() && self.pending.is_none() && !self.blocked,
            );
        }
        if self
            .adapter
            .as_ref()
            .is_some_and(|a| a.status_cached().fault.is_some())
        {
            self.detach();
            #[cfg(target_os = "macos")]
            return;
        }
        #[cfg(target_os = "macos")]
        if let Some(events) = &self.events {
            match events.changed() {
                Ok(true) => self.discover_blocking(),
                Ok(false) => {
                    if self.adapter.is_none() && self.pending.is_none() && !self.blocked {
                        let selected = self.selection.take_deferred();
                        self.attach_selected(selected);
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

enum Completion {
    Opened(Result<Box<DjiAdapter>, DjiHostError>),
    Closed(bool),
}
