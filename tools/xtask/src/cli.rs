//! Argument parsing for the xtask entry point: `sim`, `reset`,
//! `handshake`, `help`.

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
}

/// A parsed xtask invocation.
#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    /// Launch a full SITL session and supervise it.
    Sim(SimArgs),
    /// Reset the running simulation world and FC of the named backend.
    Reset(String),
    /// Produce the Alia X-Plane runtime handshake into a directory.
    Handshake(std::path::PathBuf),
    /// Run every guard pair the repository declares.
    Guards,
    /// Classify which CI matrix halves a diff against a base can reach.
    Affected {
        /// The base git reference the diff is taken against.
        base: String,
    },
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
        "reset" => Ok(Command::Reset(parse_reset(rest)?)),
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
        "help" | "--help" | "-h" => Ok(Command::Help),
        other => Err(XtaskError::Usage {
            message: format!(
                "unknown command {other:?} (expected sim, reset, handshake, affected, guards, or help)"
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

fn parse_reset(args: &[String]) -> Result<String, XtaskError> {
    let mut fc = "aviate".to_owned();
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
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
                    message: format!("unknown reset argument {other:?} (expected --fc <name>)"),
                });
            }
        }
    }
    Ok(fc)
}

fn parse_sim(args: &[String]) -> Result<SimArgs, XtaskError> {
    let mut fc = "aviate".to_owned();
    let mut profile = Profile::Simulation;
    let mut host_port: u16 = 4433;
    let mut viewer_port: u16 = 8080;
    let mut open = true;
    let mut lan = false;
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--fc" => fc = required_value(&mut iter, "--fc")?,
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
                           px4-gz (alias px4), px4-xplane, or
                           aviate-xplane
      --profile <profile>  physical | simulation (default) | oracle-only
      --port <port>        host WebTransport port (default: 4433)
      --viewer-port <port> static viewer port (default: 8080)
      --open               open the ready URL (default)
      --no-open            do not open the ready URL
      --lan                serve the session to the local network

  reset [options]
      Reset the running simulation world and restart its FC.

      --fc <name>          FC backend (default: aviate)

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

  help
      Print this help text.";

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic)]
mod tests {
    use super::{Command, Profile, USAGE, parse_args};
    use crate::error::XtaskError;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn sim_defaults_are_the_documented_ones() {
        let Command::Sim(sim) = parse_args(&args(&["sim"])).expect("parses") else {
            panic!("expected sim");
        };
        assert_eq!(sim.fc, "aviate");
        assert_eq!(sim.profile, Profile::Simulation);
        assert_eq!(sim.host_port, 4433);
        assert_eq!(sim.viewer_port, 8080);
        assert!(sim.open, "the browser opens by default");
        assert!(!sim.lan, "the session serves loopback only by default");
    }

    #[test]
    fn help_groups_each_command_once_and_matches_parser_defaults() {
        let sim_heading = "  sim [options]\n";
        let reset_heading = "  reset [options]\n";
        let handshake_heading = "  handshake [options]\n";
        let affected_heading = "  affected --base <ref>\n";
        let help_heading = "  help\n";

        for heading in [
            sim_heading,
            reset_heading,
            handshake_heading,
            affected_heading,
            help_heading,
        ] {
            assert_eq!(USAGE.matches(heading).count(), 1, "heading {heading:?}");
        }

        let sim_start = USAGE.find(sim_heading).expect("sim heading");
        let reset_start = USAGE.find(reset_heading).expect("reset heading");
        let handshake_start = USAGE.find(handshake_heading).expect("handshake heading");
        let help_start = USAGE.find(help_heading).expect("help heading");
        let affected_start = USAGE.find(affected_heading).expect("affected heading");
        assert!(sim_start < reset_start && reset_start < handshake_start);
        assert!(handshake_start < affected_start && affected_start < help_start);

        let sim_help = &USAGE[sim_start..reset_start];
        assert!(sim_help.contains("aviate-gz (alias aviate, default)"));
        assert!(sim_help.contains("px4-xplane"));
        assert!(sim_help.contains("simulation (default)"));
        assert!(sim_help.contains("default: 4433"));
        assert!(sim_help.contains("default: 8080"));
        assert!(sim_help.contains("--open"));
        assert!(sim_help.contains("--no-open"));
        assert!(sim_help.contains("--lan"));

        let reset_help = &USAGE[reset_start..handshake_start];
        assert!(reset_help.contains("--fc <name>"));
        assert!(reset_help.contains("default: aviate"));
        assert!(!reset_help.contains("--profile"));

        let handshake_help = &USAGE[handshake_start..help_start];
        assert!(handshake_help.contains("--out-dir <dir>"));
        assert!(handshake_help.contains("target/xtask-sim"));
    }

    #[test]
    fn handshake_out_dir_parses_and_malformed_flags_fail_closed() {
        assert_eq!(
            parse_args(&args(&["handshake"])).expect("default handshake parses"),
            Command::Handshake(std::path::PathBuf::from("target/xtask-sim"))
        );
        assert_eq!(
            parse_args(&args(&["handshake", "--out-dir", "/tmp/attempt"]))
                .expect("explicit out-dir parses"),
            Command::Handshake(std::path::PathBuf::from("/tmp/attempt"))
        );
        for malformed in [
            args(&["handshake", "--out-dir"]),
            args(&["handshake", "--dir", "/tmp/attempt"]),
        ] {
            assert!(matches!(
                parse_args(&malformed),
                Err(XtaskError::Usage { .. })
            ));
        }
    }

    #[test]
    fn no_open_suppresses_the_browser() {
        let Command::Sim(sim) = parse_args(&args(&["sim", "--no-open", "--lan"])).expect("parses")
        else {
            panic!("expected sim");
        };
        assert!(!sim.open);
        assert!(sim.lan, "--lan is honored");
    }

    #[test]
    fn reset_fc_selection_parses_and_malformed_flags_fail_closed() {
        assert_eq!(
            parse_args(&args(&["reset"])).expect("default reset parses"),
            Command::Reset("aviate".to_owned())
        );
        assert_eq!(
            parse_args(&args(&["reset", "--fc", "px4-gz"])).expect("explicit PX4 reset parses"),
            Command::Reset("px4-gz".to_owned())
        );
        for malformed in [
            args(&["reset", "--fc"]),
            args(&["reset", "--backend", "px4-gz"]),
        ] {
            assert!(matches!(
                parse_args(&malformed),
                Err(XtaskError::Usage { .. })
            ));
        }
    }

    #[test]
    fn sim_flags_parse_and_unknown_values_fail_closed() {
        let Command::Sim(sim) = parse_args(&args(&[
            "sim",
            "--profile",
            "oracle-only",
            "--port",
            "5000",
            "--viewer-port",
            "9000",
            "--open",
        ]))
        .expect("parses") else {
            panic!("expected sim");
        };
        assert_eq!(sim.profile, Profile::OracleOnly);
        assert_eq!(sim.host_port, 5000);
        assert_eq!(sim.viewer_port, 9000);
        assert!(sim.open);

        let refusal = parse_args(&args(&["sim", "--profile", "simulaton"]));
        assert!(matches!(refusal, Err(XtaskError::UnknownProfile { .. })));
        let refusal = parse_args(&args(&["sim", "--port", "banana"]));
        assert!(matches!(refusal, Err(XtaskError::Usage { .. })));
        let refusal = parse_args(&args(&["fly"]));
        assert!(matches!(refusal, Err(XtaskError::Usage { .. })));
    }

    #[test]
    fn port_zero_is_rejected_on_both_port_flags() {
        // Port 0 would bind an ephemeral port that readiness checks and
        // the printed viewer URL never learn about.
        for flag in ["--port", "--viewer-port"] {
            let refusal = parse_args(&args(&["sim", flag, "0"]));
            assert!(
                matches!(refusal, Err(XtaskError::Usage { .. })),
                "{flag} 0 must be refused"
            );
        }
    }
}
