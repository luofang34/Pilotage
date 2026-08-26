//! The X-Plane simulator as a deployment: where it is installed, which
//! aircraft and channel map a session selects, and how a session arms
//! the bridge without an operator click.
//!
//! X-Plane is operator-owned, like a physical bench device. A launcher
//! may START it when it is absent, but it is never a managed stage,
//! never a stale-process pattern, and a reset never kills it. Both
//! flight-controller backends share this module, so the two cannot
//! drift apart on which aircraft an airframe name means.

use std::path::{Path, PathBuf};

use crate::error::XtaskError;
use crate::output::print_line;

mod readiness;

#[cfg(test)]
pub(super) use readiness::{
    classify_xplane_probe_status, run_xplane_prepare_sequence, verify_weather_plugin_blocking,
    xplane_running_with_program_blocking,
};
pub(super) use readiness::{prepare_xplane_runtime_blocking, xplane_running_blocking};

/// X-Plane's UDP command/data port on the local machine.
const XPLANE_UDP_PORT: u16 = 49000;

/// One selectable X-Plane airframe: the PX4 `SYS_AUTOSTART` id, the
/// aircraft file X-Plane loads, and the px4xplane `config_name` whose
/// channel map matches that aircraft.
#[derive(Debug)]
pub(super) struct Airframe {
    /// Selector value accepted from `PILOTAGE_XPLANE_AIRFRAME`.
    pub(super) key: &'static str,
    /// PX4 airframe autostart id (ships in PX4's `init.d-posix`).
    pub(super) sys_autostart: &'static str,
    /// Aircraft path relative to the X-Plane root.
    pub(super) acf_path: &'static str,
    /// The px4xplane config section for this aircraft.
    pub(super) config_name: &'static str,
}

/// Airframes this backend can fly without purchased add-ons: the
/// packaged QuadTailsitter and two stock X-Plane 12 aircraft.
pub(super) const AIRFRAMES: [Airframe; 3] = [
    Airframe {
        key: "qtailsitter",
        sys_autostart: "5021",
        acf_path: "Aircraft/Extra Aircraft/QuadTailsitter/QuadTailsitter.acf",
        config_name: "QuadTailsitter",
    },
    Airframe {
        key: "cessna172",
        sys_autostart: "5001",
        acf_path: "Aircraft/Laminar Research/Cessna 172 SP/Cessna_172SP.acf",
        config_name: "Cessna172",
    },
    Airframe {
        key: "alia250",
        sys_autostart: "5020",
        acf_path: "Aircraft/Laminar Research/BETA Technologies Alia-250/ALIA-250.acf",
        config_name: "Alia250",
    },
];

/// Resolves `PILOTAGE_XPLANE_AIRFRAME` fail-closed; absent means the
/// packaged QuadTailsitter (the hover profile closest to the gz x500).
pub(super) fn selected_airframe() -> Result<&'static Airframe, XtaskError> {
    let value = std::env::var("PILOTAGE_XPLANE_AIRFRAME").ok();
    airframe_for(value.as_deref())
}

/// The testable core of [`selected_airframe`].
pub(super) fn airframe_for(key: Option<&str>) -> Result<&'static Airframe, XtaskError> {
    let key = key.unwrap_or("qtailsitter");
    AIRFRAMES
        .iter()
        .find(|airframe| airframe.key == key)
        .ok_or_else(|| XtaskError::Usage {
            message: format!(
                "unknown PILOTAGE_XPLANE_AIRFRAME {key:?} (expected qtailsitter, \
                 cessna172, or alia250)"
            ),
        })
}

/// Where the X-Plane installation lives: `XPLANE_ROOT`, else the first
/// entry of the official installer registry that holds `X-Plane.app`.
pub(super) fn xplane_root() -> Result<PathBuf, XtaskError> {
    let registry = home_dir().join("Library/Preferences/x-plane_install_12.txt");
    let content = std::fs::read_to_string(&registry).unwrap_or_default();
    xplane_root_from(std::env::var_os("XPLANE_ROOT").map(PathBuf::from), &content).ok_or(
        XtaskError::MissingArtifact {
            what: "X-Plane 12 installation",
            path: registry,
            hint: "install X-Plane 12 or set XPLANE_ROOT to its folder",
        },
    )
}

/// The testable core of [`xplane_root`]: an explicit override wins;
/// otherwise the first registry entry that holds `X-Plane.app`.
pub(super) fn xplane_root_from(
    explicit: Option<PathBuf>,
    registry_content: &str,
) -> Option<PathBuf> {
    if let Some(root) = explicit {
        return Some(root);
    }
    registry_content
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(PathBuf::from)
        .find(|candidate| candidate.join("X-Plane.app").is_dir())
}

/// The user's home directory (`HOME`, else `/`).
pub(super) fn home_dir() -> PathBuf {
    std::env::var_os("HOME").map_or_else(|| PathBuf::from("/"), PathBuf::from)
}

/// Validates the plugins and the aircraft this session needs, each with
/// an actionable hint.
pub(super) fn validate_xplane_install(root: &Path, airframe: &Airframe) -> Result<(), XtaskError> {
    let bridge = root.join("Resources/plugins/px4xplane/64/mac.xpl");
    if !bridge.is_file() {
        return Err(XtaskError::MissingArtifact {
            what: "px4xplane bridge plugin",
            path: bridge,
            hint: "run scripts/build-xplane-plugins.sh",
        });
    }
    let autoflight = root.join("Resources/plugins/PilotageAutoFlight/64/mac.xpl");
    if !autoflight.is_file() {
        return Err(XtaskError::MissingArtifact {
            what: "PilotageAutoFlight plugin",
            path: autoflight,
            hint: "run scripts/build-xplane-plugins.sh",
        });
    }
    let camera = root.join("Resources/plugins/PilotageCamera/64/mac.xpl");
    if !camera.is_file() {
        return Err(XtaskError::MissingArtifact {
            what: "PilotageCamera plugin",
            path: camera,
            hint: "run scripts/build-xplane-plugins.sh",
        });
    }
    let weather = root.join("Resources/plugins/PilotageWeather/64/mac.xpl");
    if !weather.is_file() {
        return Err(XtaskError::MissingArtifact {
            what: "Pilotage X-Plane weather plugin",
            path: weather,
            hint: "run scripts/build-xplane-plugins.sh",
        });
    }
    let aircraft = root.join(airframe.acf_path);
    if !aircraft.is_file() {
        return Err(XtaskError::MissingArtifact {
            what: "X-Plane aircraft for the selected airframe",
            path: aircraft,
            hint: "run scripts/build-xplane-plugins.sh, or pick another \
                   PILOTAGE_XPLANE_AIRFRAME",
        });
    }
    Ok(())
}

/// Where the plugin build script's content stamp lives.
const XPLANE_PLUGINS_STAMP: &str = "target/xtask-stamps/xplane-plugins-stopped-simulator.stamp";

/// The working-tree inputs whose content decides plugin staleness.
const XPLANE_PLUGINS_SOURCES: [&str; 5] = [
    "sim/xplane/autoflight",
    "sim/xplane/camera",
    "sim/xplane/trial",
    "sim/xplane/weather",
    "scripts/build-xplane-plugins.sh",
];

/// Whether the prepare step changed the installed plugin files.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum XPlanePluginInstall {
    /// The content stamp and all installed artifacts were current.
    Current,
    /// The prepare step built and installed the plugin set.
    Rebuilt,
}

pub(super) fn plugin_install_required(
    plugins_current: bool,
    simulator_running: bool,
) -> Result<bool, XtaskError> {
    if plugins_current {
        return Ok(false);
    }
    if simulator_running {
        return Err(XtaskError::SimulatorCapability {
            capability: "loaded X-Plane plugin set",
            detail: "plugin files need an update while X-Plane is running; stop X-Plane and retry"
                .to_owned(),
        });
    }
    Ok(true)
}

/// Builds and installs the required X-Plane plugins when an input changes.
/// The caller checks all required artifacts before it starts a session.
pub(super) fn ensure_xplane_plugins_blocking(
    repo_root: &Path,
    simulator_running: bool,
) -> Result<XPlanePluginInstall, XtaskError> {
    use crate::session::preflight::stamp;
    let installed = xplane_root()
        .map(|root| {
            [
                "Resources/plugins/px4xplane/64/mac.xpl",
                "Resources/plugins/PilotageAutoFlight/64/mac.xpl",
                "Resources/plugins/PilotageCamera/64/mac.xpl",
                // The manifest as well as the plugin: the trial plugin states
                // which bridge build it was built against, and a verifier that
                // cannot read that has nothing to check the bridge against.
                "Resources/plugins/PilotageTrial/64/mac.xpl",
                "Resources/plugins/PilotageTrial/build-manifest.json",
                "Resources/plugins/PilotageWeather/64/mac.xpl",
            ]
            .iter()
            .all(|path| root.join(path).is_file())
        })
        .unwrap_or(false);
    let current = stamp::source_stamp(repo_root, &XPLANE_PLUGINS_SOURCES, &[]);
    let stamp_path = repo_root.join(XPLANE_PLUGINS_STAMP);
    let stored = stamp::read_stamp(&stamp_path);
    let plugins_current =
        stamp::artifact_is_fresh(installed, stored.as_deref(), current.as_deref());
    if !plugin_install_required(plugins_current, simulator_running)? {
        return Ok(XPlanePluginInstall::Current);
    }
    print_line("building and installing the X-Plane plugins...");
    run_plugin_build_blocking(repo_root, Path::new("bash"))?;
    record_plugin_install_stamp(&stamp_path, current.as_deref(), xplane_running_blocking()?)?;
    print_line("X-Plane plugins installed");
    Ok(XPlanePluginInstall::Rebuilt)
}

pub(super) fn record_plugin_install_stamp(
    stamp_path: &Path,
    current: Option<&str>,
    simulator_running: bool,
) -> Result<(), XtaskError> {
    plugin_install_required(false, simulator_running)?;
    if let Some(current) = current {
        crate::session::preflight::stamp::write_stamp(stamp_path, current);
    }
    Ok(())
}

/// Run the plugin build with an explicit program for failure testing.
pub(super) fn run_plugin_build_blocking(
    repo_root: &Path,
    program: &Path,
) -> Result<(), XtaskError> {
    let status = std::process::Command::new(program)
        .arg(repo_root.join("scripts/build-xplane-plugins.sh"))
        .current_dir(repo_root)
        .status()
        .map_err(|source| XtaskError::Io {
            context: "starting the X-Plane plugin build",
            source,
        })?;
    if !status.success() {
        return Err(XtaskError::CommandFailed {
            context: "X-Plane plugin build and install",
            status: status.to_string(),
        });
    }
    Ok(())
}

/// Points the installed px4xplane config at the selected airframe's
/// channel map. Best-effort: a missing config fails closed later, at
/// connect time, inside the plugin.
/// Sets the bridge's ground-stationary sensor contract on or off.
///
/// The contract fabricates zero-motion sensors while the vehicle sits
/// still — a crutch PX4's EKF wants on a jittery simulator floor.
/// Aviate's estimator must see REAL sensors from boot to touchdown: the
/// fabricated-to-live handoff at liftoff is a step change in every
/// channel at the worst possible moment, and a missing measurement is
/// the client's to display as missing, never to paper over. Each
/// backend states its choice at prepare time, so lane switches cannot
/// inherit the other flight controller's crutch.
pub(super) fn set_ground_sensor_contract(
    root: &Path,
    enabled: bool,
    simulator_running: bool,
) -> Result<(), XtaskError> {
    let value = if enabled { "true" } else { "false" };
    for config in [
        root.join("Resources/plugins/px4xplane/64/config.ini"),
        root.join("config.ini"),
    ] {
        let Ok(content) = std::fs::read_to_string(&config) else {
            continue;
        };
        let rewritten: String = content
            .lines()
            .map(|line| {
                let key = line.trim_start();
                if key.starts_with("ground_stationary_accel_guard_enabled") {
                    format!("ground_stationary_accel_guard_enabled = {value}")
                } else if key.starts_with("ground_stationary_kinematics_guard_enabled") {
                    format!("ground_stationary_kinematics_guard_enabled = {value}")
                } else {
                    line.to_owned()
                }
            })
            .collect::<Vec<_>>()
            .join(
                "
",
            )
            + "
";
        write_config_or_refuse(
            &config,
            &content,
            &rewritten,
            simulator_running,
            "the ground-stationary guard setting",
        )?;
    }
    Ok(())
}

/// Writes one bridge configuration file, or refuses when the simulator is
/// already up and the file would have to change.
///
/// The bridge reads its configuration when it loads. Rewriting the file under
/// a running simulator changes nothing the run will use, and the launcher then
/// digests the file it wrote — putting a claim in the trial document that the
/// running bridge does not match. A launcher that finds X-Plane already
/// running does not get to choose the configuration any more than it gets to
/// choose the aircraft: it checks, and says what the operator has to do.
///
/// A file that already says the right thing is not a change, so the common
/// case of a simulator already running the right configuration still passes.
fn write_config_or_refuse(
    config: &Path,
    content: &str,
    rewritten: &str,
    simulator_running: bool,
    setting: &'static str,
) -> Result<(), XtaskError> {
    if rewritten == content {
        return Ok(());
    }
    if simulator_running {
        return Err(XtaskError::SimulatorCapability {
            capability: "px4xplane bridge configuration matching this session",
            detail: format!(
                "X-Plane is already running and {setting} in {} does not match what this \
                 session needs. The bridge reads its configuration when it loads, so \
                 rewriting it now would not reach the running bridge. Reload the bridge \
                 configuration in X-Plane, or quit X-Plane and let this launcher start it.",
                config.display(),
            ),
        });
    }
    if std::fs::write(config, rewritten).is_err() {
        print_line("could not update the px4xplane config; check config.ini");
    }
    Ok(())
}

/// The aircraft a running X-Plane most recently loaded, from its own log,
/// or `None` when the log says nothing this reader understands.
///
/// A launcher that STARTS X-Plane also chooses the aircraft, so the two
/// agree by construction. A launcher that finds X-Plane already running
/// chooses only the bridge configuration, and the operator chose the
/// aircraft — possibly a session ago, for another airframe.
#[must_use]
pub(super) fn loaded_aircraft(log: &str) -> Option<String> {
    const MARKER: &str = "Loading airplane number 0 with ";
    log.lines().rev().find_map(|line| {
        line.split_once(MARKER)
            .map(|(_, acf)| acf.trim().to_owned())
    })
}

/// Refuses a bridge configuration that names a different aircraft than the
/// one X-Plane has loaded.
///
/// The bridge answers the flight controller's connection and then drops it,
/// and neither side says why: the flight controller retries until its
/// readiness deadline and the session fails with nothing pointing at the
/// aircraft. The mismatch is knowable before anything starts.
pub(super) fn verify_loaded_aircraft(root: &Path, airframe: &Airframe) -> Result<(), XtaskError> {
    let Ok(log) = std::fs::read_to_string(root.join("Log.txt")) else {
        // No log to read is not a mismatch. This check only ever turns a
        // silent failure into a named one; it never invents one.
        return Ok(());
    };
    let Some(loaded) = loaded_aircraft(&log) else {
        return Ok(());
    };
    if loaded == airframe.acf_path {
        return Ok(());
    }
    Err(XtaskError::SimulatorCapability {
        capability: "X-Plane aircraft matching the selected airframe",
        detail: format!(
            "X-Plane has {loaded} loaded and this session needs {} for airframe {}. \
             Load that aircraft in X-Plane, or select the airframe that matches what \
             is loaded with PILOTAGE_XPLANE_AIRFRAME.",
            airframe.acf_path, airframe.key,
        ),
    })
}

pub(super) fn set_active_config_name(
    root: &Path,
    airframe: &Airframe,
    simulator_running: bool,
) -> Result<(), XtaskError> {
    for config in [
        root.join("Resources/plugins/px4xplane/64/config.ini"),
        root.join("config.ini"),
    ] {
        let Ok(content) = std::fs::read_to_string(&config) else {
            continue;
        };
        let rewritten: String = content
            .lines()
            .map(|line| {
                if line.trim_start().starts_with("config_name") {
                    format!("config_name = {}", airframe.config_name)
                } else {
                    line.to_owned()
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
            + "\n";
        write_config_or_refuse(
            &config,
            &content,
            &rewritten,
            simulator_running,
            "config_name",
        )?;
    }
    Ok(())
}

/// Sends one X-Plane `CMND` datagram to the local simulator. Fire and
/// forget: UDP, no reply channel.
pub(super) fn send_xplane_command(command: &str) {
    let Ok(socket) = std::net::UdpSocket::bind("127.0.0.1:0") else {
        return;
    };
    socket
        .send_to(&command_datagram(command), ("127.0.0.1", XPLANE_UDP_PORT))
        .ok();
}

/// X-Plane's `CMND` wire shape: the 5-byte `CMND\0` prologue, the
/// command path, and a closing NUL.
pub(super) fn command_datagram(command: &str) -> Vec<u8> {
    let mut payload = Vec::with_capacity(5 + command.len() + 1);
    payload.extend_from_slice(b"CMND\0");
    payload.extend_from_slice(command.as_bytes());
    payload.push(0);
    payload
}
