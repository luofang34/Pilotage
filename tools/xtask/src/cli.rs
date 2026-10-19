//! Argument parsing for the xtask entry point: `sim`, `reset`,
//! `handshake`, `affected`, `guards`, `serve-viewer`, `help`.

use crate::error::XtaskError;

/// Which session profile the host runs (mirrors the host's fail-closed
/// `PILOTAGE_AVIATE_PROFILE` vocabulary exactly).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Profile {
    /// FC estimate + FC state; no truth source.
    Physical,
    /// Estimate + FC state, plus the truth oracle when attachable.
    Simulation,
    /// Truth stream only; no uplink, no operational control.
    OracleOnly,
}

impl Profile {
    /// The value passed through to `PILOTAGE_AVIATE_PROFILE`.
    pub fn as_env_value(self) -> &'static str {
        match self {
            Profile::Physical => "physical",
            Profile::Simulation => "simulation",
            Profile::OracleOnly => "oracle-only",
        }
    }

    fn parse(value: &str) -> Result<Self, XtaskError> {
        match value {
            "physical" => Ok(Profile::Physical),
            "simulation" => Ok(Profile::Simulation),
            "oracle-only" => Ok(Profile::OracleOnly),
            _ => Err(XtaskError::UnknownProfile {
                value: value.to_owned(),
            }),
        }
    }
}

/// Options for `cargo xtask sim`.
#[derive(Debug, PartialEq, Eq)]
pub struct SimArgs {
    /// Which FC backend to launch (`--fc`, default `aviate`).
    pub fc: String,
    /// Session profile handed to the host (`--profile`).
    pub profile: Profile,
    /// Host WebTransport port (`--port`).
    pub host_port: u16,
    /// Static viewer port (`--viewer-port`).
    pub viewer_port: u16,
    /// Open the ready URL in the default browser (default on;
    /// `--no-open` suppresses it, `--open` states it explicitly).
    pub open: bool,
    /// Serve the session to the local network (`--lan`): the viewer and
    /// its connect manifest become reachable from another device, and the
    /// native connect facts are printed.
    pub lan: bool,
    /// Simulator instance index (`--instance`, default 0). A session on
    /// another instance shifts its simulator and FC ports and isolates
    /// its simulator, so it can run beside the instance-0 session.
    pub instance: u16,
}

/// Options for `cargo xtask fleet`.
#[derive(Debug, PartialEq)]
pub struct FleetArgs {
    /// One `<backend>[:<model>]` per vehicle, in launch order
    /// (`--vehicle`, repeated).
    pub vehicles: Vec<String>,
    /// Session profile handed to every host (`--profile`).
    pub profile: Profile,
    /// The first vehicle's host port; vehicle `k` gets `host_port + k`.
    pub host_port: u16,
    /// The first vehicle's viewer port; vehicle `k` gets
    /// `viewer_port + k`.
    pub viewer_port: u16,
    /// The first vehicle's simulator instance; vehicle `k` gets
    /// `instance_base + k`.
    pub instance_base: u16,
    /// Open every vehicle's viewer URL (`--open`; off by default, since a
    /// fleet opens one tab per vehicle).
    pub open: bool,
    /// The airfield every vehicle spawns on (`--site`).
    pub site: crate::site::Site,
}

/// A parsed xtask invocation.
#[derive(Debug, PartialEq)]
pub enum Command {
    /// Launch a full SITL session and supervise it.
    Sim(SimArgs),
    /// Launch one session per vehicle and supervise them all.
    Fleet(FleetArgs),
    /// Reset the running simulation world and FC of the named backend.
    Reset {
        /// The backend.
        fc: String,
        /// The simulator instance to reset.
        instance: u16,
    },
    /// Produce the Alia X-Plane runtime handshake into a directory.
    Handshake(std::path::PathBuf),
    /// Run every guard pair the repository declares.
    Guards,
    /// Classify which CI matrix halves a diff against a base can reach.
    Affected {
        /// The base git reference the diff is taken against.
        base: String,
    },
    /// Serve a directory to the browser viewer.
    ServeViewer(crate::viewer_server::ServeViewerArgs),
    /// Print usage.
    Help,
}

/// Parses raw arguments (without the binary name).
///
/// # Errors
///
/// Returns [`XtaskError::Usage`], [`XtaskError::UnknownProfile`], or
/// [`XtaskError::UnknownBackend`] on malformed input; backend existence
/// is validated later by the backend registry.
pub fn parse_args(args: &[String]) -> Result<Command, XtaskError> {
    let Some((command, rest)) = args.split_first() else {
        return Ok(Command::Help);
    };
    match command.as_str() {
        "sim" => Ok(Command::Sim(parse_sim(rest)?)),
        "fleet" => Ok(Command::Fleet(parse_fleet(rest)?)),
        "reset" => {
            let (fc, instance) = parse_reset(rest)?;
            Ok(Command::Reset { fc, instance })
        }
        "handshake" => Ok(Command::Handshake(parse_handshake(rest)?)),
        "guards" => {
            if let Some(extra) = rest.first() {
                return Err(XtaskError::Usage {
                    message: format!("unknown guards argument {extra:?}"),
                });
            }
            Ok(Command::Guards)
        }
        "affected" => Ok(Command::Affected {
            base: parse_affected(rest)?,
        }),
        "serve-viewer" => Ok(Command::ServeViewer(crate::viewer_server::parse_args(
            rest,
        )?)),
        "help" | "--help" | "-h" => Ok(Command::Help),
        other => Err(XtaskError::Usage {
            message: format!(
                "unknown command {other:?} (expected sim, fleet, reset, handshake, affected, guards, serve-viewer, or help)"
            ),
        }),
    }
}

fn parse_handshake(args: &[String]) -> Result<std::path::PathBuf, XtaskError> {
    let mut out_dir = std::path::PathBuf::from(crate::session::SESSION_DIR);
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--out-dir" => {
                out_dir =
                    std::path::PathBuf::from(iter.next().ok_or_else(|| XtaskError::Usage {
                        message: "--out-dir requires a value".to_owned(),
                    })?);
            }
            other => {
                return Err(XtaskError::Usage {
                    message: format!(
                        "unknown handshake argument {other:?} (expected --out-dir <dir>)"
                    ),
                });
            }
        }
    }
    Ok(out_dir)
}

fn parse_affected(args: &[String]) -> Result<String, XtaskError> {
    let mut base: Option<String> = None;
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--base" => {
                base = Some(
                    iter.next()
                        .ok_or_else(|| XtaskError::Usage {
                            message: "--base requires a value".to_owned(),
                        })?
                        .clone(),
                );
            }
            other => {
                return Err(XtaskError::Usage {
                    message: format!("unknown affected argument {other:?} (expected --base <ref>)"),
                });
            }
        }
    }
    base.ok_or_else(|| XtaskError::Usage {
        message: "affected requires --base <ref>".to_owned(),
    })
}

fn parse_reset(args: &[String]) -> Result<(String, u16), XtaskError> {
    let mut fc = "aviate".to_owned();
    let mut instance: u16 = 0;
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--instance" => instance = required_number(&mut iter, "--instance")?,
            "--fc" => {
                fc = iter
                    .next()
                    .ok_or_else(|| XtaskError::Usage {
                        message: "--fc requires a value".to_owned(),
                    })?
                    .clone();
            }
            other => {
                return Err(XtaskError::Usage {
                    message: format!(
                        "unknown reset argument {other:?} (expected --fc <name> or --instance <n>)"
                    ),
                });
            }
        }
    }
    Ok((fc, instance))
}

fn parse_sim(args: &[String]) -> Result<SimArgs, XtaskError> {
    let mut fc = "aviate".to_owned();
    let mut profile = Profile::Simulation;
    let mut host_port: u16 = 4433;
    let mut viewer_port: u16 = 8080;
    let mut open = true;
    let mut lan = false;
    let mut instance: u16 = 0;
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--fc" => fc = required_value(&mut iter, "--fc")?,
            "--instance" => instance = required_number(&mut iter, "--instance")?,
            "--profile" => profile = Profile::parse(&required_value(&mut iter, "--profile")?)?,
            "--port" => host_port = required_port(&mut iter, "--port")?,
            "--viewer-port" => viewer_port = required_port(&mut iter, "--viewer-port")?,
            "--open" => open = true,
            "--no-open" => open = false,
            "--lan" => lan = true,
            other => {
                return Err(XtaskError::Usage {
                    message: format!("unknown sim flag {other:?}"),
                });
            }
        }
    }
    Ok(SimArgs {
        fc,
        profile,
        host_port,
        viewer_port,
        open,
        lan,
        instance,
    })
}

fn parse_fleet(args: &[String]) -> Result<FleetArgs, XtaskError> {
    let mut fleet = FleetArgs {
        vehicles: Vec::new(),
        profile: Profile::Simulation,
        host_port: 4433,
        viewer_port: 8080,
        instance_base: 0,
        open: false,
        site: crate::site::DEFAULT_SITE,
    };
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--vehicle" => fleet.vehicles.push(required_value(&mut iter, "--vehicle")?),
            "--profile" => {
                fleet.profile = Profile::parse(&required_value(&mut iter, "--profile")?)?;
            }
            "--port" => fleet.host_port = required_port(&mut iter, "--port")?,
            "--viewer-port" => fleet.viewer_port = required_port(&mut iter, "--viewer-port")?,
            "--instance-base" => {
                fleet.instance_base = required_number(&mut iter, "--instance-base")?;
            }
            "--site" => {
                fleet.site = crate::site::Site::parse(&required_value(&mut iter, "--site")?)?
            }
            "--open" => fleet.open = true,
            "--no-open" => fleet.open = false,
            other => {
                return Err(XtaskError::Usage {
                    message: format!("unknown fleet flag {other:?}"),
                });
            }
        }
    }
    if fleet.vehicles.is_empty() {
        return Err(XtaskError::Usage {
            message: "fleet needs at least one --vehicle <backend>[:<model>]".to_owned(),
        });
    }
    // Every vehicle's port and instance must exist: a fleet whose last
    // vehicle wraps past 65535 would collide with its first.
    let last = u16::try_from(fleet.vehicles.len() - 1).unwrap_or(u16::MAX);
    for (flag, base) in [
        ("--port", fleet.host_port),
        ("--viewer-port", fleet.viewer_port),
        ("--instance-base", fleet.instance_base),
    ] {
        if base.checked_add(last).is_none() {
            return Err(XtaskError::Usage {
                message: format!("{flag} {base} leaves no room for {} vehicles", last + 1),
            });
        }
    }
    Ok(fleet)
}

fn required_number(
    iter: &mut std::slice::Iter<'_, String>,
    flag: &'static str,
) -> Result<u16, XtaskError> {
    let raw = required_value(iter, flag)?;
    raw.parse().map_err(|_| XtaskError::Usage {
        message: format!("{flag} expects a number, got {raw:?}"),
    })
}

fn required_value(
    iter: &mut std::slice::Iter<'_, String>,
    flag: &'static str,
) -> Result<String, XtaskError> {
    iter.next().cloned().ok_or_else(|| XtaskError::Usage {
        message: format!("{flag} requires a value"),
    })
}

fn required_port(
    iter: &mut std::slice::Iter<'_, String>,
    flag: &'static str,
) -> Result<u16, XtaskError> {
    let raw = required_value(iter, flag)?;
    let port: u16 = raw.parse().map_err(|_| XtaskError::Usage {
        message: format!("{flag} expects a port number, got {raw:?}"),
    })?;
    // Port 0 asks the OS for an ephemeral port, but readiness checks and
    // the printed viewer URL would keep using the literal 0 — nothing
    // reports the allocated port back, so the session could never be
    // reached. Reject it rather than print a URL that cannot work.
    if port == 0 {
        return Err(XtaskError::Usage {
            message: format!("{flag} must be 1-65535: port 0 (ephemeral) cannot be advertised"),
        });
    }
    Ok(port)
}

/// The `help` text.
pub const USAGE: &str = "\
cargo xtask <command> [options]

commands:
  sim [options]
      Launch a full SITL session, print its ready URL, supervise it, and
      tear it down on ctrl-c.

      --fc <name>          FC backend: aviate-gz (alias aviate, default),
                           px4-gz (alias px4), px4-xplane,
                           aviate-xplane, ardupilot-sitl (alias
                           ardupilot), or betaflight-sitl (alias
                           betaflight); px4-gz takes a vehicle model
                           as px4-gz:<model>: x500_gimbal (default) or
                           rc_cessna
      --profile <profile>  physical | simulation (default) | oracle-only
      --port <port>        host WebTransport port (default: 4433)
      --viewer-port <port> static viewer port (default: 8080)
      --instance <n>       simulator instance (default: 0); another
                           instance shifts the simulator and FC ports and
                           runs beside instance 0 on other ports
      --open               open the ready URL (default)
      --no-open            do not open the ready URL
      --lan                serve the session to the local network

  fleet --vehicle <backend>[:<model>] [--vehicle ...] [options]
      Launch one session per vehicle, each with its own simulator,
      host, and viewer, spawned on one airfield: a fixed-wing on the
      runway threshold facing down the runway, every other vehicle on
      the grass beside it, 40 m apart. Writes
      target/xtask-fleet/fleet.json and serves the first vehicle's
      clients/web/session.json; supervises every session and tears all
      of them down on ctrl-c.

      --vehicle <spec>     a backend with an optional model, as for
                           sim --fc (repeat once per vehicle)
      --profile <profile>  physical | simulation (default) | oracle-only
      --port <port>        first host port (default: 4433); vehicle k
                           gets port + k
      --viewer-port <port> first viewer port (default: 8080); vehicle k
                           gets viewer-port + k
      --instance-base <n>  first simulator instance (default: 0);
                           vehicle k gets instance-base + k
      --site <site>        the airfield: n51 (default, Solberg-Hunterdon,
                           runway 04), zurich (PX4's default home), or
                           latitude_deg,longitude_deg,elevation_m,heading_deg
                           for a threshold, its ground elevation (m
                           MSL), and the runway's true course
      --open               open every vehicle's viewer URL
      --no-open            open none (default)

  reset [options]
      Reset the running simulation world and restart its FC.

      --fc <name>          FC backend (default: aviate)
      --instance <n>       simulator instance (default: 0); px4-gz
                           resets only that instance, and only while an
                           xtask supervisor runs it

  handshake [options]
      Produce the Alia X-Plane runtime handshake document and print its
      path, without launching a session. For harnesses that run the
      flight controller themselves; X-Plane must be running with the
      trial plugin loaded.

      --out-dir <dir>      directory to write into (default:
                           target/xtask-sim)

  affected --base <ref>
      Classify which CI matrix halves the diff base...HEAD can reach,
      printing one key=value line per answer with reason lines above.
      Fails open: an unclassifiable change answers everything=true.

  guards
      Discover every scripts/check-<name>.sh with its
      scripts/test-check-<name>.sh self-test, run each pair, and name
      every failing guard.

  serve-viewer --dir <dir> --port <port> [--bind <addr>]
      Serve <dir> statically to the browser viewer with caching disabled,
      on 127.0.0.1 unless --bind says otherwise; port 0 picks a free port
      and prints it.

  help
      Print this help text.";

#[cfg(test)]
mod tests;
