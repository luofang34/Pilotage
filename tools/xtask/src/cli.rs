//! Argument parsing for the xtask entry point: `sim`, `reset`, `help`.

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
    /// Open the ready URL in the default browser (`--open`).
    pub open: bool,
}

/// A parsed xtask invocation.
#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    /// Launch a full SITL session and supervise it.
    Sim(SimArgs),
    /// Reset the running simulation world and FC.
    Reset,
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
        "reset" => {
            if let Some(extra) = rest.first() {
                return Err(XtaskError::Usage {
                    message: format!("reset takes no arguments (got {extra:?})"),
                });
            }
            Ok(Command::Reset)
        }
        "help" | "--help" | "-h" => Ok(Command::Help),
        other => Err(XtaskError::Usage {
            message: format!("unknown command {other:?} (expected sim, reset, or help)"),
        }),
    }
}

fn parse_sim(args: &[String]) -> Result<SimArgs, XtaskError> {
    let mut fc = "aviate".to_owned();
    let mut profile = Profile::Simulation;
    let mut host_port: u16 = 4433;
    let mut viewer_port: u16 = 8080;
    let mut open = false;
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--fc" => fc = required_value(&mut iter, "--fc")?,
            "--profile" => profile = Profile::parse(&required_value(&mut iter, "--profile")?)?,
            "--port" => host_port = required_port(&mut iter, "--port")?,
            "--viewer-port" => viewer_port = required_port(&mut iter, "--viewer-port")?,
            "--open" => open = true,
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
cargo xtask <command>

commands:
  sim    launch a full SITL session (simulator + FC + host + viewer),
         print the ready URL, supervise, and tear down on ctrl-c
         --fc <name>          FC backend (default: aviate)
         --profile <p>        physical | simulation (default) | oracle-only
         --port <n>           host WebTransport port (default: 4433)
         --viewer-port <n>    static viewer port (default: 8080)
         --open               open the ready URL in the default browser
  reset  reset the running simulation world and restart the FC
  help   print this text";

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic)]
mod tests {
    use super::{Command, Profile, parse_args};
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
        assert!(!sim.open);
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
