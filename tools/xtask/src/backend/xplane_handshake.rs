//! The runtime handshake the Alia's flight controller will not start without.
//!
//! Aviate binds every Alia run to a verified runtime: the flight controller
//! refuses to start unless it is handed a document naming which simulator,
//! which aircraft file, which bridge build and which bridge configuration the
//! run flew against. Pilotage is the verifier that document names, so this is
//! where it is produced.
//!
//! Nothing here is asserted. The trial plugin loaded inside X-Plane dials this
//! listener and states its own identity; the artifacts on disk are digested
//! here; and the document is written only once the two agree. A digest typed
//! in by hand would forge exactly the binding the mechanism exists to make,
//! and every trial recorded afterwards would carry a fabricated identity.
//!
//! The file is single-use and private by contract: the flight controller
//! claims it by deleting it, and refuses one that any other account could have
//! read or substituted.

use std::io::Write as _;
use std::os::unix::fs::OpenOptionsExt as _;
use std::path::{Path, PathBuf};
use std::time::Duration;

use pilotage_trial::Digest;
use pilotage_xplane_trial::{
    ExpectedArtifact, ExpectedXPlaneIdentity, TrialPluginBuildManifest, VerifiedXPlaneIdentity,
    XPlaneTrialListener,
};
use sha2::{Digest as _, Sha256};

use crate::error::XtaskError;

/// Where the trial plugin dials. Fixed rather than configurable: the plugin
/// is built with it, so a launcher that chose its own port would simply never
/// be found.
const LISTEN: &str = "127.0.0.1:45991";

/// The verifier this document is issued by, as Aviate's reader checks it.
const VERIFIER_ID: &str = "pilotage-xplane-trial-v1";

/// How the flight controller reaches the bridge inside X-Plane.
const BRIDGE_ENDPOINT: &str = "127.0.0.1:4560";

/// What the vehicle's own simulator model declares about itself.
///
/// Read from Aviate rather than restated here. The flight controller checks
/// this document against that same model, so a value copied into this repo
/// would be a second opinion about a fact the model owns — and the first thing
/// to drift when the model changed. Only the fields the handshake carries are
/// decoded; the rest of the preset is the model's business.
#[derive(serde::Deserialize)]
struct SimulatorModel {
    simulator_id: String,
    aircraft_id: String,
    aircraft_file_digest: String,
    bridge_protocol: String,
    motor_count: u8,
    sample_rate_hz: u32,
    lane_order: [u8; 4],
}

/// Long enough for a reader to bring X-Plane to the aircraft, short enough
/// that a plugin which is not loaded fails the launch instead of hanging it.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(90);

/// The document, exactly as Aviate's reader decodes it.
#[derive(serde::Serialize)]
struct RuntimeHandshake<'a> {
    schema_version: u16,
    verifier_id: &'static str,
    session_binding_digest: String,
    bridge_endpoint: &'static str,
    bridge_protocol: &'a str,
    bridge_build_digest: String,
    bridge_config_digest: String,
    simulator_id: &'a str,
    aircraft_id: &'a str,
    aircraft_file_digest: String,
    sample_rate_hz: u32,
    motor_count: u8,
    lane_order: [u8; 4],
}

/// Verifies the running X-Plane and writes the handshake it earns.
///
/// Returns the path the flight controller should be pointed at. The caller
/// owns nothing afterwards: the flight controller consumes the file.
pub(crate) fn produce_blocking(
    xplane_root: &Path,
    model_preset: &Path,
    out_dir: &Path,
) -> Result<PathBuf, XtaskError> {
    let model = read_model(model_preset)?;
    let plugins = xplane_root.join("Resources/plugins");
    let trial_plugin = plugins.join("PilotageTrial/64/mac.xpl");
    let bridge_plugin = plugins.join("px4xplane/64/mac.xpl");
    let bridge_config = plugins.join("px4xplane/64/config.ini");
    let manifest_path = plugins.join("PilotageTrial/build-manifest.json");
    let aircraft = aircraft_path(xplane_root)?;

    for (what, path) in [
        ("X-Plane trial plugin", &trial_plugin),
        ("X-Plane bridge plugin", &bridge_plugin),
        ("X-Plane bridge configuration", &bridge_config),
        ("X-Plane trial build manifest", &manifest_path),
    ] {
        if !path.is_file() {
            return Err(XtaskError::MissingArtifact {
                what,
                path: path.clone(),
                hint: "install the Pilotage X-Plane plugins into this X-Plane",
            });
        }
    }

    let manifest = TrialPluginBuildManifest::from_json_file_blocking(&manifest_path)
        .map_err(|source| verification_failed(format!("trial build manifest: {source}")))?;
    let bridge_digest = file_digest(&bridge_plugin)?;
    if bridge_digest != manifest.bridge_plugin_digest {
        return Err(verification_failed(
            "the installed bridge plugin is not the one the trial plugin was built against"
                .to_owned(),
        ));
    }
    let aircraft_digest = file_digest(&aircraft)?;
    let config_digest = file_digest(&bridge_config)?;

    let expected = ExpectedXPlaneIdentity {
        aircraft: ExpectedArtifact::new(&aircraft, aircraft_digest),
        trial_plugin: ExpectedArtifact::new(&trial_plugin, file_digest(&trial_plugin)?),
        bridge_plugin: ExpectedArtifact::new(&bridge_plugin, bridge_digest),
        bridge_config: ExpectedArtifact::new(&bridge_config, config_digest),
        trial_source_build_id: manifest.trial_source_build_id,
        // The model contract this run is bound to. Nothing upstream states one,
        // so it is derived from the artifacts that decide what the vehicle
        // actually is: this aircraft file, under this bridge configuration,
        // for this airframe. A constant here would bind every run to the same
        // declared model however the aircraft or its configuration changed.
        simulator_model_digest: model_contract_digest(
            aircraft_digest,
            config_digest,
            &model.aircraft_id,
        ),
    };

    let listener = XPlaneTrialListener::bind_blocking(LISTEN)
        .map_err(|source| verification_failed(format!("cannot listen on {LISTEN}: {source}")))?;
    crate::output::print_line(&format!(
        "waiting for the X-Plane trial plugin to connect on {LISTEN} ..."
    ));
    let session = listener
        .accept_verified_blocking(&expected, CONNECT_TIMEOUT)
        .map_err(|source| verification_failed(format!("{source}")))?;

    write_handshake(session.identity(), &model, out_dir)
}

/// Reads the simulator model the flight controller will check this against.
fn read_model(path: &Path) -> Result<SimulatorModel, XtaskError> {
    let text = std::fs::read_to_string(path).map_err(|source| XtaskError::Io {
        context: "read the Alia X-Plane simulator model preset",
        source,
    })?;
    let model: SimulatorModel = toml::from_str(&text)
        .map_err(|source| verification_failed(format!("simulator model preset: {source}")))?;
    model.validate()?;
    Ok(model)
}

impl SimulatorModel {
    /// Refuses a model that cannot describe a vehicle.
    ///
    /// These fields configure motor mixing on the far side of the handshake,
    /// so a preset that states nothing useful must fail the launch rather than
    /// be copied into a document the flight controller then trusts. The
    /// build manifest beside this one is already validated this way; a file
    /// that decides which motor is which deserves at least as much.
    fn validate(&self) -> Result<(), XtaskError> {
        let refuse = |detail: String| Err(verification_failed(detail));
        if self.simulator_id.is_empty() || self.aircraft_id.is_empty() {
            return refuse("the model names no simulator or no aircraft".to_owned());
        }
        if self.bridge_protocol.is_empty() {
            return refuse("the model names no bridge protocol".to_owned());
        }
        if self.motor_count == 0 {
            return refuse("the model states no motors".to_owned());
        }
        if self.sample_rate_hz == 0 {
            return refuse("the model states no sample rate".to_owned());
        }
        // A permutation, not just four numbers: a repeated lane silently sends
        // two motors the same command and leaves another unaddressed.
        let mut lanes = self.lane_order;
        lanes.sort_unstable();
        if lanes != [0, 1, 2, 3] {
            return refuse(format!(
                "the model's lane order {:?} is not a permutation of the four lanes",
                self.lane_order
            ));
        }
        if usize::from(self.motor_count) != self.lane_order.len() {
            return refuse(format!(
                "the model states {} motors but orders {} lanes",
                self.motor_count,
                self.lane_order.len()
            ));
        }
        Ok(())
    }
}

/// The aircraft the Alia backend flies, inside this X-Plane.
fn aircraft_path(xplane_root: &Path) -> Result<PathBuf, XtaskError> {
    let path =
        xplane_root.join("Aircraft/Laminar Research/BETA Technologies Alia-250/ALIA-250.acf");
    if path.is_file() {
        return Ok(path);
    }
    Err(XtaskError::MissingArtifact {
        what: "the Alia 250 aircraft",
        path,
        hint: "install the BETA Technologies Alia-250 into this X-Plane",
    })
}

/// What decides which vehicle this run flew: the aircraft, the bridge
/// configuration that shapes it, and the airframe it is flown as.
fn model_contract_digest(aircraft: Digest, bridge_config: Digest, aircraft_id: &str) -> Digest {
    let mut hasher = Sha256::new();
    hasher.update(b"pilotage.xplane.model-contract.v1\0");
    hasher.update(aircraft.as_bytes());
    hasher.update(bridge_config.as_bytes());
    hasher.update(aircraft_id.as_bytes());
    Digest::from_bytes(hasher.finalize().into())
}

/// Writes the document where only this account can read it.
///
/// Created with the private mode rather than chmod-ed into it: a file that is
/// briefly world-readable is a file that was briefly readable.
fn write_handshake(
    verified: &VerifiedXPlaneIdentity,
    model: &SimulatorModel,
    out_dir: &Path,
) -> Result<PathBuf, XtaskError> {
    // The aircraft the reader verified must be the aircraft the model was
    // written for. Same file, or the run is not the run the model describes.
    if !verified
        .aircraft_digest
        .to_string()
        .eq_ignore_ascii_case(model.aircraft_file_digest.trim())
    {
        return Err(verification_failed(format!(
            "the loaded aircraft is not the one the simulator model declares \
             (running {}, model {})",
            verified.aircraft_digest, model.aircraft_file_digest
        )));
    }
    let document = RuntimeHandshake {
        schema_version: 1,
        verifier_id: VERIFIER_ID,
        session_binding_digest: verified.binding_digest.to_string(),
        bridge_endpoint: BRIDGE_ENDPOINT,
        bridge_protocol: &model.bridge_protocol,
        bridge_build_digest: verified.bridge_plugin_digest.to_string(),
        bridge_config_digest: verified.bridge_config_digest.to_string(),
        simulator_id: &model.simulator_id,
        aircraft_id: &model.aircraft_id,
        aircraft_file_digest: verified.aircraft_digest.to_string(),
        sample_rate_hz: model.sample_rate_hz,
        motor_count: model.motor_count,
        lane_order: model.lane_order,
    };
    let text = toml::to_string(&document)
        .map_err(|source| verification_failed(format!("cannot encode the handshake: {source}")))?;

    std::fs::create_dir_all(out_dir).map_err(|source| XtaskError::Io {
        context: "create the runtime handshake directory",
        source,
    })?;
    let path = out_dir.join("runtime-handshake.toml");
    // A stale document from a previous launch would be claimed instead of
    // this one, binding the run to a simulator that is no longer running.
    std::fs::remove_file(&path).ok();
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&path)
        .map_err(|source| XtaskError::Io {
            context: "write the runtime handshake",
            source,
        })?;
    file.write_all(text.as_bytes())
        .and_then(|()| file.sync_all())
        .map_err(|source| XtaskError::Io {
            context: "write the runtime handshake",
            source,
        })?;
    Ok(path)
}

fn file_digest(path: &Path) -> Result<Digest, XtaskError> {
    let bytes = std::fs::read(path).map_err(|source| XtaskError::Io {
        context: "read an X-Plane runtime artifact to digest it",
        source,
    })?;
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    Ok(Digest::from_bytes(hasher.finalize().into()))
}

fn verification_failed(detail: String) -> XtaskError {
    XtaskError::SimulatorCapability {
        capability: "a verified X-Plane runtime identity",
        detail,
    }
}

#[cfg(test)]
mod tests;
