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
use crate::readiness::stage_log;

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
const XPLANE_PLUGINS_STAMP: &str = "target/xtask-stamps/xplane-plugins.stamp";

/// The working-tree inputs whose content decides plugin staleness.
const XPLANE_PLUGINS_SOURCES: [&str; 3] = [
    "sim/xplane/autoflight",
    "sim/xplane/camera",
    "scripts/build-xplane-plugins.sh",
];

/// Best-effort, content-stamped build + install of the two X-Plane
/// plugins and the packaged aircraft. Non-fatal by contract: `plan`
/// fails closed with hints when a required artifact is still absent.
pub(super) fn ensure_xplane_plugins(repo_root: &Path) {
    use crate::session::preflight::stamp;
    let installed = xplane_root()
        .map(|root| {
            root.join("Resources/plugins/PilotageAutoFlight/64/mac.xpl")
                .is_file()
        })
        .unwrap_or(false);
    let current = stamp::source_stamp(repo_root, &XPLANE_PLUGINS_SOURCES, &[]);
    let stamp_path = repo_root.join(XPLANE_PLUGINS_STAMP);
    let stored = stamp::read_stamp(&stamp_path);
    if stamp::artifact_is_fresh(installed, stored.as_deref(), current.as_deref()) {
        return;
    }
    print_line("building and installing the X-Plane plugins...");
    let built = std::process::Command::new("bash")
        .arg(repo_root.join("scripts/build-xplane-plugins.sh"))
        .current_dir(repo_root)
        .status();
    match built {
        Ok(status) if status.success() => {
            if let Some(current) = current {
                stamp::write_stamp(&stamp_path, &current);
            }
            print_line("X-Plane plugins installed");
        }
        Ok(_) | Err(_) => print_line(
            "X-Plane plugin build failed (see build-xplane-plugins output); \
             the session will fail closed if a required plugin is missing",
        ),
    }
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
pub(super) fn set_ground_sensor_contract(root: &Path, enabled: bool) {
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
            .join("
")
            + "
";
        if rewritten != content && std::fs::write(&config, rewritten).is_err() {
            print_line("could not update the px4xplane ground-contract keys; check config.ini");
        }
    }
}

pub(super) fn set_active_config_name(root: &Path, airframe: &Airframe) {
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
        if rewritten != content && std::fs::write(&config, rewritten).is_err() {
            print_line("could not update the px4xplane config_name; check config.ini");
        }
    }
}

/// True when an X-Plane simulator process is running on this machine.
pub(super) fn xplane_running() -> bool {
    std::process::Command::new("pgrep")
        .args(["-f", "X-Plane.app/Contents/MacOS/X-Plane"])
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

/// Starts X-Plane detached with the PilotageAutoFlight environment. The
/// process deliberately outlives the session: booting X-Plane costs
/// minutes, and the operator owns its window.
pub(super) fn launch_xplane(root: &Path, airframe: &Airframe, log_dir: &Path) {
    print_line("starting X-Plane (a cold boot takes minutes)...");
    // A fresh simulator gets a fresh parking spot: the reset script
    // captures the vehicle's home position once per X-Plane boot and
    // teleports back to it on every reset.
    if let Some(home) = std::env::var_os("HOME") {
        std::fs::remove_file(
            std::path::PathBuf::from(home).join(".pilotage/xplane-home.json"),
        )
        .ok();
    }
    let log = std::fs::File::create(stage_log(log_dir, "xplane"))
        .ok()
        .map_or(std::process::Stdio::null(), std::process::Stdio::from);
    let spawned = std::process::Command::new(root.join("X-Plane.app/Contents/MacOS/X-Plane"))
        .arg("--window=1400x900")
        .arg("--pref:_show_qfl_on_start=0")
        .current_dir(root)
        .env("PILOTAGE_XPLANE_ACF", airframe.acf_path)
        .env("PILOTAGE_XPLANE_CONNECT", "1")
        // A cold simulator start plus a flight-controller boot can
        // exceed the bridge's default one-minute connect window, and an
        // expired window needs an operator click. Wait without a
        // deadline instead (a local px4xplane patch).
        .env("PX4XPLANE_CONNECT_TIMEOUT_S", "0")
        .stdout(log)
        .stderr(std::process::Stdio::null())
        .spawn();
    if spawned.is_err() {
        print_line("could not start X-Plane; start it by hand and rerun");
    }
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
