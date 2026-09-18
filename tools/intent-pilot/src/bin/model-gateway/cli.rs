//! Command-line options of the model gateway.

/// Usage text.
pub(crate) const USAGE: &str = "\
model-gateway: the model port over HTTP, for a client that cannot start a process

USAGE:
  model-gateway --model-cmd <command> [--listen <address:port>] [--allow-origin <origin>]...

  --model-cmd     Shell command that starts a model adapter (JSON lines on stdin and stdout).
  --listen        Listen address. Default: 127.0.0.1:8098
  --allow-origin  A page origin that can call the gateway. Repeat it for each origin.
                  Default: http://localhost:8099 and http://127.0.0.1:8099";

/// The origins of the viewer that the session launcher serves.
const DEFAULT_ORIGINS: [&str; 2] = ["http://localhost:8099", "http://127.0.0.1:8099"];

/// Parsed options.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Options {
    /// Shell command that starts the model adapter.
    pub model_cmd: String,
    /// Listen address.
    pub listen: String,
    /// Page origins that can call the gateway.
    pub allowed_origins: Vec<String>,
}

/// What the command line asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Invocation {
    /// Print the usage text.
    Help,
    /// Serve.
    Run(Options),
}

/// Parses the arguments that follow the program name.
pub(crate) fn parse(args: impl IntoIterator<Item = String>) -> Result<Invocation, String> {
    let mut model_cmd = None;
    let mut listen = "127.0.0.1:8098".to_owned();
    let mut allowed_origins = Vec::new();
    let mut args = args.into_iter();
    while let Some(flag) = args.next() {
        let mut value = || args.next().ok_or_else(|| format!("{flag} needs a value"));
        match flag.as_str() {
            "--help" | "-h" => return Ok(Invocation::Help),
            "--model-cmd" => model_cmd = Some(value()?),
            "--listen" => listen = value()?,
            "--allow-origin" => allowed_origins.push(value()?.trim_end_matches('/').to_owned()),
            other => return Err(format!("unrecognized argument {other}")),
        }
    }
    if allowed_origins.is_empty() {
        allowed_origins = DEFAULT_ORIGINS.map(str::to_owned).to_vec();
    }
    Ok(Invocation::Run(Options {
        model_cmd: model_cmd.ok_or("--model-cmd is required")?,
        listen,
        allowed_origins,
    }))
}

#[cfg(test)]
#[allow(clippy::panic)]
mod tests {
    use super::{Invocation, parse};

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|text| (*text).to_owned()).collect()
    }

    #[test]
    fn the_defaults_are_loopback_and_the_launcher_viewer() {
        let Ok(Invocation::Run(options)) = parse(args(&["--model-cmd", "adapter"])) else {
            panic!("the options must parse");
        };
        assert_eq!(options.listen, "127.0.0.1:8098");
        assert_eq!(
            options.allowed_origins,
            ["http://localhost:8099", "http://127.0.0.1:8099"]
        );
    }

    #[test]
    fn a_named_origin_replaces_the_defaults() {
        let parsed = parse(args(&[
            "--model-cmd",
            "adapter",
            "--allow-origin",
            "http://viewer:8099/",
        ]));
        let Ok(Invocation::Run(options)) = parsed else {
            panic!("the options must parse");
        };
        assert_eq!(options.allowed_origins, ["http://viewer:8099"]);
    }

    #[test]
    fn a_missing_adapter_command_is_a_usage_error() {
        assert!(parse(args(&["--listen", "127.0.0.1:1"])).is_err());
        assert!(parse(args(&["--model-cmd"])).is_err());
    }
}
