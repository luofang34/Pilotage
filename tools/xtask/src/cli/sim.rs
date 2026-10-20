//! Simulator launch options and explicit network access.

use std::path::PathBuf;

use super::{Profile, SimArgs, required_number, required_port, required_timeout, required_value};
use crate::error::XtaskError;

pub(super) fn parse_sim(args: &[String]) -> Result<SimArgs, XtaskError> {
    let mut sim = SimArgs {
        fc: "aviate".to_owned(),
        profile: Profile::Simulation,
        host_port: 4433,
        viewer_port: 8080,
        open: true,
        lan: false,
        join_token_file: None,
        instance: 0,
        ready_timeout_s: crate::backend::DEFAULT_READY_TIMEOUT_S,
        sim_camera: false,
    };
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--fc" => sim.fc = required_value(&mut iter, "--fc")?,
            "--instance" => sim.instance = required_number(&mut iter, "--instance")?,
            "--profile" => sim.profile = Profile::parse(&required_value(&mut iter, "--profile")?)?,
            "--port" => sim.host_port = required_port(&mut iter, "--port")?,
            "--viewer-port" => sim.viewer_port = required_port(&mut iter, "--viewer-port")?,
            "--open" => sim.open = true,
            "--no-open" => sim.open = false,
            "--lan" => sim.lan = true,
            "--join-token-file" => {
                sim.join_token_file = Some(token_path(&required_value(
                    &mut iter,
                    "--join-token-file",
                )?)?);
            }
            "--ready-timeout" => sim.ready_timeout_s = required_timeout(&mut iter)?,
            "--camera" => {
                sim.sim_camera =
                    crate::camera::parse_choice(&required_value(&mut iter, "--camera")?)?;
            }
            other => {
                return Err(XtaskError::Usage {
                    message: format!("unknown sim flag {other:?}"),
                });
            }
        }
    }
    if sim.lan && sim.join_token_file.is_none() {
        return Err(XtaskError::Usage {
            message: "--lan requires --join-token-file with an absolute local path".to_owned(),
        });
    }
    Ok(sim)
}

fn token_path(value: &str) -> Result<PathBuf, XtaskError> {
    let path = PathBuf::from(value);
    if !path.is_absolute() {
        return Err(XtaskError::Usage {
            message: "--join-token-file requires an absolute local path".to_owned(),
        });
    }
    Ok(path)
}
