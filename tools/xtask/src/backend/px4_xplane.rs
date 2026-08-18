//! The PX4 + X-Plane SITL backend: X-Plane runs as an external,
//! operator-owned desktop application (like a physical bench device),
//! while the px4xplane X-Plane plugin bridges the standard MAVLink HIL
//! stream (`HIL_SENSOR`/`HIL_GPS` in, `HIL_ACTUATOR_CONTROLS` out) to
//! PX4 over TCP 4560. PX4 selects that path with `PX4_SIMULATOR=xplane`
//! and dials the plugin, so stage order is forgiving: PX4 retries until
//! the plugin listens. The adapter side is untouched — the PX4 adapter
//! is FDM-agnostic, so this backend only plans different stages.
//!
//! The launcher never manages the X-Plane process: it is not a stage,
//! never a stale-process pattern, and reset never kills it. When
//! X-Plane is not running, `prepare` starts it detached with the
//! PilotageAutoFlight environment so the flight loads and the SITL
//! listener arms without operator input; when it is already running,
//! `prepare` arms the listener with an X-Plane UDP `CMND` datagram.

use std::path::{Path, PathBuf};

use super::{SessionContext, SimBackend, Stage};
use crate::cli::Profile;
use crate::error::XtaskError;
use crate::output::print_line;
use crate::process::ProcessSpec;
use crate::readiness::{Readiness, stage_log};

/// X-Plane's UDP command/data port on the local machine.
const XPLANE_UDP_PORT: u16 = 49000;

/// One selectable X-Plane airframe: the PX4 `SYS_AUTOSTART` id, the
/// aircraft file X-Plane loads, and the px4xplane `config_name` whose
/// channel map matches that aircraft.
#[derive(Debug)]
struct Airframe {
    /// Selector value accepted from `PILOTAGE_XPLANE_AIRFRAME`.
    key: &'static str,
    /// PX4 airframe autostart id (ships in PX4's `init.d-posix`).
    sys_autostart: &'static str,
    /// Aircraft path relative to the X-Plane root.
    acf_path: &'static str,
    /// The px4xplane config section for this aircraft.
    config_name: &'static str,
}

/// Airframes this backend can fly without purchased add-ons: the
/// packaged QuadTailsitter and two stock X-Plane 12 aircraft.
const AIRFRAMES: [Airframe; 3] = [
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
fn selected_airframe() -> Result<&'static Airframe, XtaskError> {
    let value = std::env::var("PILOTAGE_XPLANE_AIRFRAME").ok();
    airframe_for(value.as_deref())
}

/// The testable core of [`selected_airframe`].
fn airframe_for(key: Option<&str>) -> Result<&'static Airframe, XtaskError> {
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

/// The PX4 + X-Plane SITL backend.
#[derive(Debug)]
pub struct Px4XPlane;

impl SimBackend for Px4XPlane {
    fn name(&self) -> &'static str {
        "px4-xplane"
    }

    fn host_adapter(&self) -> &'static str {
        "px4"
    }

    fn host_env(&self, ctx: &SessionContext) -> Vec<(String, String)> {
        vec![
            ("PILOTAGE_PX4_PROFILE".to_owned(), "simulation".to_owned()),
            // No gimbal device exists in the X-Plane HIL bridge; the
            // scope must not be advertised. X-Plane exposes no camera
            // topic either, so the vehicle view comes from the
            // in-simulator camera plugin, which dials the host.
            ("PILOTAGE_PX4_CAMERA".to_owned(), "xplane-plugin".to_owned()),
            (
                "PILOTAGE_RESET_CMD".to_owned(),
                ctx.repo_root
                    .join("scripts/reset-xplane-sim.sh")
                    .display()
                    .to_string(),
            ),
        ]
    }

    fn plan(&self, ctx: &SessionContext) -> Result<Vec<Stage>, XtaskError> {
        if ctx.profile != Profile::Simulation {
            return Err(XtaskError::Usage {
                message: format!(
                    "the px4-xplane backend supports only --profile simulation (got {:?})",
                    ctx.profile
                ),
            });
        }
        let airframe = selected_airframe()?;
        let root = xplane_root()?;
        validate_xplane_install(&root, airframe)?;
        let px4 = super::px4_gz::px4_dir(&ctx.repo_root);
        let binary = px4.join("build/px4_sitl_default/bin/px4");
        if !binary.is_file() {
            return Err(XtaskError::MissingArtifact {
                what: "PX4 SITL binary",
                path: binary,
                hint: "build it: make px4_sitl in the PX4-Autopilot checkout",
            });
        }
        Ok(vec![px4_stage(ctx, &px4, &binary, airframe)?])
    }

    fn prepare(&self, ctx: &SessionContext) -> Result<(), XtaskError> {
        ensure_xplane_plugins(&ctx.repo_root);
        let Ok(airframe) = selected_airframe() else {
            // plan() re-runs the same resolution and reports the error.
            return Ok(());
        };
        let Ok(root) = xplane_root() else {
            return Ok(());
        };
        set_active_config_name(&root, airframe);
        if xplane_running() {
            // A running X-Plane never inherits the autoflight
            // environment; arm the SITL listener directly. The command
            // is the idempotent px4xplane/connect (a local px4xplane
            // patch): a no-op when already armed or connected.
            // Best-effort: the datagram is lost when no flight is
            // loaded yet.
            print_line("X-Plane is already running; arming the SITL listener...");
            send_xplane_command("px4xplane/connect");
            print_line("if PX4 cannot connect: Plugins > PX4 X-Plane > Connect to SITL");
        } else {
            launch_xplane(&root, airframe, &ctx.log_dir);
        }
        Ok(())
    }

    fn stale_process_patterns(&self) -> Vec<&'static str> {
        // X-Plane is deliberately absent: the launcher never owns it.
        vec!["bin/px4"]
    }

    fn reset(&self, repo_root: &Path) -> Result<(), XtaskError> {
        let script = repo_root.join("scripts/reset-xplane-sim.sh");
        let status = std::process::Command::new("bash")
            .arg(&script)
            .env("PX4_DIR", super::px4_gz::px4_dir(repo_root))
            .status()
            .map_err(|source| XtaskError::Io {
                context: "running the X-Plane reset script",
                source,
            })?;
        if status.success() {
            Ok(())
        } else {
            Err(XtaskError::CommandFailed {
                context: "X-Plane reset script",
                status: status.to_string(),
            })
        }
    }
}

/// Where the X-Plane installation lives: `XPLANE_ROOT`, else the first
/// entry of the official installer registry that holds `X-Plane.app`.
fn xplane_root() -> Result<PathBuf, XtaskError> {
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
fn xplane_root_from(explicit: Option<PathBuf>, registry_content: &str) -> Option<PathBuf> {
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
fn home_dir() -> PathBuf {
    std::env::var_os("HOME").map_or_else(|| PathBuf::from("/"), PathBuf::from)
}

/// Validates the plugins and the aircraft this session needs, each with
/// an actionable hint.
fn validate_xplane_install(root: &Path, airframe: &Airframe) -> Result<(), XtaskError> {
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
fn ensure_xplane_plugins(repo_root: &Path) {
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
fn set_active_config_name(root: &Path, airframe: &Airframe) {
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
fn xplane_running() -> bool {
    std::process::Command::new("pgrep")
        .args(["-f", "X-Plane.app/Contents/MacOS/X-Plane"])
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

/// Starts X-Plane detached with the PilotageAutoFlight environment. The
/// process deliberately outlives the session: booting X-Plane costs
/// minutes, and the operator owns its window.
fn launch_xplane(root: &Path, airframe: &Airframe, log_dir: &Path) {
    print_line("starting X-Plane (a cold boot takes minutes)...");
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
fn send_xplane_command(command: &str) {
    let Ok(socket) = std::net::UdpSocket::bind("127.0.0.1:0") else {
        return;
    };
    socket
        .send_to(&command_datagram(command), ("127.0.0.1", XPLANE_UDP_PORT))
        .ok();
}

/// X-Plane's `CMND` wire shape: the 5-byte `CMND\0` prologue, the
/// command path, and a closing NUL.
fn command_datagram(command: &str) -> Vec<u8> {
    let mut payload = Vec::with_capacity(5 + command.len() + 1);
    payload.extend_from_slice(b"CMND\0");
    payload.extend_from_slice(command.as_bytes());
    payload.push(0);
    payload
}

/// The PX4 SITL stage: `PX4_SIMULATOR=xplane` routes the startup script
/// to `simulator_mavlink`, which dials the plugin on TCP 4560 and
/// retries until it answers. The readiness needle appears only after
/// that link is up, so the deadline covers an X-Plane cold boot.
fn px4_stage(
    ctx: &SessionContext,
    px4: &Path,
    binary: &Path,
    airframe: &Airframe,
) -> Result<Stage, XtaskError> {
    // A rootfs of its own: SITL parameters persist in the rootfs, and a
    // stale parameters.bson from another simulator's airframe would
    // override this airframe's defaults.
    let rootfs = px4.join("build/px4_sitl_default/rootfs-xplane");
    std::fs::create_dir_all(&rootfs).map_err(|source| XtaskError::Io {
        context: "creating the PX4 X-Plane rootfs directory",
        source,
    })?;
    Ok(Stage {
        spec: ProcessSpec {
            name: "flight-controller",
            program: binary.display().to_string(),
            args: vec![
                px4.join("build/px4_sitl_default/etc").display().to_string(),
                "-s".to_owned(),
                "etc/init.d-posix/rcS".to_owned(),
                "-d".to_owned(),
            ],
            cwd: Some(rootfs),
            env: vec![
                ("PX4_SIMULATOR".to_owned(), "xplane".to_owned()),
                ("PX4_SIM_HOSTNAME".to_owned(), "127.0.0.1".to_owned()),
                (
                    "PX4_SYS_AUTOSTART".to_owned(),
                    airframe.sys_autostart.to_owned(),
                ),
            ],
            remove_env: vec![],
            log_path: stage_log(&ctx.log_dir, "flight-controller"),
        },
        readiness: Readiness::LogContains {
            needle: "Startup script returned successfully",
            timeout_s: 420,
        },
    })
}

#[cfg(test)]
mod tests;
