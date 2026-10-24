//! `cargo xtask mission`: a mission fleet from a fleet file. Each aircraft
//! is a simulated multicopter host; each section is a coordinator over its
//! aircraft, and the command lead is a coordinator over the sections
//! (ADR-0052). The hosts start bottom up, since a lead names its members'
//! certificates, are supervised together, and stop together.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::cli::MissionArgs;
use crate::error::XtaskError;
use crate::output::print_line;
use crate::process::{ManagedChild, ProcessSpec};
use crate::readiness::{Readiness, ReadySignal, await_ready};
use crate::session::fleet::{FleetEntry, FleetManifest};
use crate::session::{SUPERVISE_INTERVAL, prepared_log_dir, repo_root, spawn_cancel_listener};

mod plan;
mod rehearse;

use plan::{Launch, MissionFile};

/// The fleet's working directory, relative to the repository root.
const MISSION_DIR: &str = "target/xtask-mission";
/// The address every mission host serves: the fleet runs on loopback.
const LOOPBACK: &str = "127.0.0.1";

/// One running host.
struct Host {
    lane: String,
    adapter: &'static str,
    child: ManagedChild,
    port: u16,
    certificate: String,
    launch: Launch,
}

/// What every host of one fleet shares.
struct Fleet {
    repo_root: PathBuf,
    dir: PathBuf,
    log_dir: PathBuf,
    token: PathBuf,
    ready_timeout_s: u64,
    instruction_model: Option<String>,
}

/// Runs the mission fleet until ctrl-c or any host's failure.
///
/// # Errors
///
/// Returns a typed [`XtaskError`] when the fleet file is unusable, the
/// fault cannot be placed, a build fails, or a host fails to start or
/// stops. Every started host is stopped on every exit.
pub async fn run_mission(args: &MissionArgs) -> Result<(), XtaskError> {
    let repo_root = repo_root()?;
    let plan_path = if args.plan.is_absolute() {
        args.plan.clone()
    } else {
        repo_root.join(&args.plan)
    };
    let file = MissionFile::load_blocking(&plan_path)?;
    let fault = match (&file.fault, args.no_fault) {
        (Some(fault), false) => {
            let spec = rehearse::fault_spec(&file, fault)?;
            print_line(&format!(
                "camera fault: {} on SIM-{} {}",
                fault.kind, fault.aircraft, fault.line
            ));
            Some((format!("SIM-{}", fault.aircraft), spec))
        }
        _ => None,
    };
    build(&repo_root)?;
    let dir = repo_root.join(MISSION_DIR);
    let fleet = Fleet {
        log_dir: prepared_log_dir(&dir.join("logs"))?,
        token: approver_token_blocking(&dir)?,
        dir,
        repo_root,
        ready_timeout_s: args.ready_timeout_s,
        instruction_model: args.instruction_model.clone(),
    };
    write_objective(&fleet, &file)?;
    let mut cancel = spawn_cancel_listener()?;
    let mut hosts = Vec::new();
    let outcome = async {
        launch_all(&fleet, &file, args, fault.as_ref(), &mut hosts, &mut cancel).await?;
        write_manifest(&fleet, &hosts)?;
        announce(&fleet, &file, &hosts);
        supervise(&mut hosts, &mut cancel).await
    }
    .await;
    stop_all(&mut hosts);
    match outcome {
        Err(XtaskError::Cancelled) => Ok(()),
        other => other,
    }
}

/// Starts every aircraft, then every section lead, then the command lead.
async fn launch_all(
    fleet: &Fleet,
    file: &MissionFile,
    args: &MissionArgs,
    fault: Option<&(String, String)>,
    hosts: &mut Vec<Host>,
    cancel: &mut tokio::sync::watch::Receiver<bool>,
) -> Result<(), XtaskError> {
    for (seed, aircraft) in (1_u32..).zip(file.aircraft()) {
        let lane = aircraft.lane();
        let mut env = aircraft_env(fleet, &lane, aircraft.launch, seed, file.airframe);
        if let Some((faulted, spec)) = fault
            && *faulted == lane
        {
            env.push(("PILOTAGE_SIM_CAMERA_FAULT".to_owned(), spec.clone()));
        }
        if let Some(decision) = &args.decision {
            env.push(("PILOTAGE_DECISION".to_owned(), decision.clone()));
        }
        if let Some(every) = args.decision_every_s {
            env.push(("PILOTAGE_DECISION_EVERY_S".to_owned(), every.to_string()));
        }
        let host = start(
            fleet,
            &lane,
            "sim-multicopter",
            aircraft.launch,
            env,
            cancel,
        )
        .await?;
        hosts.push(host);
    }
    for section in &file.sections {
        let members: Vec<String> = section.aircraft.iter().map(plan::Aircraft::lane).collect();
        let host = start_lead(
            fleet,
            &section.name,
            section.launch,
            &members,
            hosts,
            cancel,
        )
        .await?;
        print_line(&format!("{} leads {}", section.name, section.landmark));
        hosts.push(host);
    }
    let sections: Vec<String> = file.sections.iter().map(|s| s.name.clone()).collect();
    let command = &file.command;
    let host = start_lead(
        fleet,
        &command.name,
        command.launch,
        &sections,
        hosts,
        cancel,
    )
    .await?;
    hosts.push(host);
    Ok(())
}

fn aircraft_env(
    fleet: &Fleet,
    lane: &str,
    [latitude, longitude, altitude]: Launch,
    seed: u32,
    airframe: plan::Airframe,
) -> Vec<(String, String)> {
    [
        (
            "PILOTAGE_SIM_LAUNCH",
            format!("{latitude},{longitude},{altitude}"),
        ),
        ("PILOTAGE_SIM_NAME", lane.to_owned()),
        ("PILOTAGE_LANE_NAME", lane.to_owned()),
        (
            "PILOTAGE_SIM_TILE_CACHE",
            fleet.dir.join("tiles").display().to_string(),
        ),
        ("PILOTAGE_SIM_SEED", seed.to_string()),
        ("PILOTAGE_LANE_ENERGY_WH", airframe.energy_wh.to_string()),
        (
            "PILOTAGE_LANE_CRUISE_POWER_W",
            airframe.cruise_power_w.to_string(),
        ),
    ]
    .into_iter()
    .map(|(key, value)| (key.to_owned(), value))
    .collect()
}

/// One member of a lead, as the coordinator host reads it.
#[derive(serde::Serialize)]
struct MemberEntry<'a> {
    name: &'a str,
    url: String,
    cert_sha256: &'a str,
    launch: Launch,
    token_file: String,
}

/// Starts a coordinator over the running hosts named `members`.
async fn start_lead(
    fleet: &Fleet,
    lane: &str,
    launch: Launch,
    members: &[String],
    hosts: &[Host],
    cancel: &mut tokio::sync::watch::Receiver<bool>,
) -> Result<Host, XtaskError> {
    let entries: Vec<MemberEntry<'_>> = hosts
        .iter()
        .filter(|host| members.contains(&host.lane))
        .map(|host| MemberEntry {
            name: &host.lane,
            url: format!("https://{LOOPBACK}:{}/pilotage", host.port),
            cert_sha256: &host.certificate,
            launch: host.launch,
            token_file: fleet.token.display().to_string(),
        })
        .collect();
    let path = fleet.dir.join(format!("members-{lane}.json"));
    let json = serde_json::to_string_pretty(&entries).map_err(std::io::Error::other);
    json.and_then(|json| std::fs::write(&path, json + "\n"))
        .map_err(|source| XtaskError::Io {
            context: "writing a lead's member list",
            source,
        })?;
    let env = vec![
        (
            "PILOTAGE_COORDINATOR_MEMBERS".to_owned(),
            path.display().to_string(),
        ),
        ("PILOTAGE_LANE_NAME".to_owned(), lane.to_owned()),
    ];
    start(fleet, lane, "coordinator", launch, env, cancel).await
}

/// Starts one host on an ephemeral port and waits for its LISTENING line.
async fn start(
    fleet: &Fleet,
    lane: &str,
    adapter: &'static str,
    launch: Launch,
    mut env: Vec<(String, String)>,
    cancel: &mut tokio::sync::watch::Receiver<bool>,
) -> Result<Host, XtaskError> {
    if std::env::var_os("RUST_LOG").is_none() {
        env.push(("RUST_LOG".to_owned(), "info".to_owned()));
    }
    let spec = ProcessSpec {
        name: if adapter == "coordinator" {
            "lead host"
        } else {
            "aircraft host"
        },
        program: fleet
            .repo_root
            .join("target/release/pilotage-session-host")
            .display()
            .to_string(),
        args: [
            "--adapter",
            adapter,
            "--bind",
            LOOPBACK,
            "--port",
            "0",
            "--approver-token-file",
        ]
        .into_iter()
        .map(str::to_owned)
        .chain([fleet.token.display().to_string()])
        .collect(),
        cwd: Some(fleet.repo_root.clone()),
        env,
        remove_env: vec![],
        log_path: fleet.log_dir.join(format!("{lane}.log")),
    };
    let mut child = ManagedChild::spawn(&spec)?;
    let readiness = Readiness::HostListening {
        timeout_s: fleet.ready_timeout_s,
    };
    let ready = tokio::select! {
        ready = await_ready(&mut child, &readiness) => ready,
        _ = cancel.changed() => Err(XtaskError::Cancelled),
    };
    match ready {
        Ok(ReadySignal::HostListening { port, certificate }) => Ok(Host {
            lane: lane.to_owned(),
            adapter,
            child,
            port,
            certificate,
            launch,
        }),
        Ok(ReadySignal::Up) => Err(XtaskError::MissionHost {
            lane: lane.to_owned(),
            source: Box::new(XtaskError::Io {
                context: "waiting for the host to listen",
                source: std::io::Error::other("no LISTENING line was captured"),
            }),
        }),
        Err(XtaskError::Cancelled) => Err(XtaskError::Cancelled),
        Err(error) => Err(XtaskError::MissionHost {
            lane: lane.to_owned(),
            source: Box::new(error),
        }),
    }
}

/// Builds the host and the mission operator the printed commands run.
fn build(repo_root: &Path) -> Result<(), XtaskError> {
    let context = "cargo build --release -p pilotage-session-host -p mission-operator";
    let status = Command::new("cargo")
        .args(["build", "--release", "-p", "pilotage-session-host"])
        .args(["-p", "mission-operator"])
        .current_dir(repo_root)
        .status()
        .map_err(|source| XtaskError::Io {
            context: "building the mission hosts",
            source,
        })?;
    if status.success() {
        Ok(())
    } else {
        Err(XtaskError::CommandFailed {
            context,
            status: status.to_string(),
        })
    }
}

/// The approver secret every host admits a mission approver with. Kept
/// across runs, so an App already configured with it stays the approver.
fn approver_token_blocking(dir: &Path) -> Result<PathBuf, XtaskError> {
    use std::io::{Read, Write};
    use std::os::unix::fs::OpenOptionsExt;
    let io = |context: &'static str| move |source| XtaskError::Io { context, source };
    std::fs::create_dir_all(dir).map_err(io("creating the mission directory"))?;
    let path = dir.join("approver.token");
    if path.exists() {
        return Ok(path);
    }
    let mut secret = [0_u8; 32];
    std::fs::File::open("/dev/urandom")
        .and_then(|mut random| random.read_exact(&mut secret))
        .map_err(io("reading randomness for the approver secret"))?;
    let hex: String = secret.iter().map(|byte| format!("{byte:02x}")).collect();
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&path)
        .and_then(|mut out| out.write_all(format!("{hex}\n").as_bytes()))
        .map_err(io("writing the approver secret"))?;
    Ok(path)
}

/// Writes the fleet's survey objective where the App and agents read it.
fn write_objective(fleet: &Fleet, file: &MissionFile) -> Result<(), XtaskError> {
    let json = serde_json::to_string_pretty(&file.objective).map_err(std::io::Error::other);
    json.and_then(|json| std::fs::write(fleet.dir.join("objective.json"), json + "\n"))
        .map_err(|source| XtaskError::Io {
            context: "writing the mission objective",
            source,
        })
}

/// Writes the App's fleet manifest: leads first, top down, then aircraft.
fn write_manifest(fleet: &Fleet, hosts: &[Host]) -> Result<(), XtaskError> {
    let entry = |host: &Host| FleetEntry {
        name: host.lane.clone(),
        backend: host.adapter.to_owned(),
        host: LOOPBACK.to_owned(),
        port: host.port,
        cert_hash: host.certificate.clone(),
    };
    let leads = hosts.iter().rev().filter(|h| h.adapter == "coordinator");
    let aircraft = hosts.iter().filter(|h| h.adapter != "coordinator");
    let manifest = FleetManifest {
        sessions: leads.chain(aircraft).map(entry).collect(),
    };
    let json = serde_json::to_string_pretty(&manifest).map_err(std::io::Error::other);
    json.and_then(|json| std::fs::write(fleet.dir.join("fleet.json"), json + "\n"))
        .map_err(|source| XtaskError::Io {
            context: "writing the App fleet manifest",
            source,
        })
}

/// Prints the fleet and the commands that open it.
fn announce(fleet: &Fleet, file: &MissionFile, hosts: &[Host]) {
    let dir = fleet.dir.display();
    let token = fleet.token.display();
    let leads: Vec<&Host> = hosts
        .iter()
        .rev()
        .filter(|h| h.adapter == "coordinator")
        .collect();
    print_line(&format!(
        "mission fleet up: {} over {} sections, {} aircraft",
        file.command.name,
        file.sections.len(),
        hosts.len() - leads.len()
    ));
    let mut bindings: Vec<String> = leads
        .iter()
        .map(|lead| format!("--pilotage-join-token-file {}={token}", lead.lane))
        .collect();
    let mut interpreter = String::new();
    if let Some(url) = &fleet.instruction_model {
        bindings.push(format!("--pilotage-instruction-model {url}"));
        interpreter = format!(" --instruction-model {url}");
    }
    print_line("App (approver on every lead), from the Sokoly-App checkout:");
    print_line(&format!(
        "  cargo run --release -p sokoly-app -- --pilotage-fleet {dir}/fleet.json \
         --mission-objective {dir}/objective.json --pilotage-survey-dir {dir}/survey {}",
        bindings.join(" ")
    ));
    let operator = fleet.repo_root.join("target/release/mission-operator");
    for lead in &leads {
        print_line(&format!(
            "agent on {}: {} --host https://{LOOPBACK}:{}/pilotage --cert {} --name agent --human{interpreter} agent",
            lead.lane,
            operator.display(),
            lead.port,
            lead.certificate
        ));
    }
    print_line(&format!("logs: {}", fleet.log_dir.display()));
}

/// Watches every host until ctrl-c or the first host that stops.
async fn supervise(
    hosts: &mut Vec<Host>,
    cancel: &mut tokio::sync::watch::Receiver<bool>,
) -> Result<(), XtaskError> {
    loop {
        tokio::select! {
            _ = cancel.changed() => {
                print_line("");
                print_line("stopping the mission fleet...");
                return Ok(());
            }
            () = tokio::time::sleep(SUPERVISE_INTERVAL) => {
                check_hosts(hosts)?;
            }
        }
    }
}

/// Checks every host once. An aircraft that stopped is reported and left out: its link
/// to its lead is lost, and the lead keeps the mission, as a lost aircraft in the field
/// would leave it. A lead that stopped ends the fleet.
fn check_hosts(hosts: &mut Vec<Host>) -> Result<(), XtaskError> {
    let mut stopped = Vec::new();
    for host in hosts.iter_mut() {
        match host.child.check_running() {
            Ok(()) => {}
            Err(error) if host.adapter != "coordinator" => {
                print_line(&format!(
                    "aircraft {} stopped; its lead keeps the mission: {error}",
                    host.lane
                ));
                stopped.push(host.lane.clone());
            }
            Err(error) => {
                return Err(XtaskError::MissionHost {
                    lane: host.lane.clone(),
                    source: Box::new(error),
                });
            }
        }
    }
    hosts.retain(|host| !stopped.contains(&host.lane));
    Ok(())
}

/// Stops every host, leads first.
fn stop_all(hosts: &mut Vec<Host>) {
    while let Some(mut host) = hosts.pop() {
        print_line(&format!("stopping {}...", host.lane));
        host.child.terminate_group();
    }
}

#[cfg(test)]
mod tests;
