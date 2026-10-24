//! Parser tests: documented defaults, fail-closed flags, and help text
//! that matches the parser.

#![allow(clippy::expect_used, clippy::panic)]

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
    assert_eq!(sim.instance, 0, "one session per machine is the default");
    assert!(!sim.lan, "the session serves loopback only by default");
    assert_eq!(sim.join_token_file, None);
    assert!(!sim.sim_camera, "the backend's own camera by default");
}

#[test]
fn the_camera_stage_is_chosen_by_name_and_unknown_cameras_fail_closed() {
    let Command::Sim(sim) = parse_args(&args(&["sim", "--camera", "maplibre"])).expect("parses")
    else {
        panic!("expected sim");
    };
    assert!(sim.sim_camera);
    let Command::Fleet(fleet) = parse_args(&args(&[
        "fleet",
        "--vehicle",
        "px4",
        "--camera",
        "maplibre",
    ]))
    .expect("parses") else {
        panic!("expected fleet");
    };
    assert!(fleet.sim_camera);
    assert!(parse_args(&args(&["sim", "--camera", "gazebo"])).is_err());
    assert!(parse_args(&args(&["sim", "--camera"])).is_err());
    assert!(USAGE.contains("--camera <camera>"));
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
    assert!(sim_help.contains("ardupilot-sitl"));
    assert!(sim_help.contains("betaflight-sitl"));
    assert!(sim_help.contains("simulation (default)"));
    assert!(sim_help.contains("default: 4433"));
    assert!(sim_help.contains("default: 8080"));
    assert!(sim_help.contains("--open"));
    assert!(sim_help.contains("--no-open"));
    assert!(sim_help.contains("--lan"));
    assert!(sim_help.contains("--instance <n>"));

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
    let Command::Sim(sim) = parse_args(&args(&[
        "sim",
        "--no-open",
        "--lan",
        "--join-token-file",
        "/private/join-code",
    ]))
    .expect("parses") else {
        panic!("expected sim");
    };
    assert!(!sim.open);
    assert!(sim.lan, "--lan is honored");
}

#[test]
fn lan_requires_an_explicit_absolute_join_code_file() {
    for flags in [
        args(&["sim", "--lan"]),
        args(&["sim", "--lan", "--join-token-file"]),
        args(&["sim", "--lan", "--join-token-file", "join-code"]),
        args(&["sim", "--join-token-file", ""]),
    ] {
        assert!(matches!(parse_args(&flags), Err(XtaskError::Usage { .. })));
    }
    let Command::Sim(sim) = parse_args(&args(&[
        "sim",
        "--join-token-file",
        "/private/operator-code",
        "--lan",
    ]))
    .expect("explicit LAN admission") else {
        panic!("expected sim");
    };
    assert_eq!(
        sim.join_token_file,
        Some(std::path::PathBuf::from("/private/operator-code"))
    );
    assert!(USAGE.contains("--join-token-file <path>"));
}

#[test]
fn reset_fc_selection_parses_and_malformed_flags_fail_closed() {
    assert_eq!(
        parse_args(&args(&["reset"])).expect("default reset parses"),
        Command::Reset {
            fc: "aviate".to_owned(),
            instance: 0
        }
    );
    assert_eq!(
        parse_args(&args(&["reset", "--fc", "px4-gz"])).expect("explicit PX4 reset parses"),
        Command::Reset {
            fc: "px4-gz".to_owned(),
            instance: 0
        }
    );
    assert_eq!(
        parse_args(&args(&["reset", "--fc", "px4-gz", "--instance", "7"])).expect("parses"),
        Command::Reset {
            fc: "px4-gz".to_owned(),
            instance: 7
        }
    );
    for malformed in [
        args(&["reset", "--fc", "px4-gz", "--instance", "seven"]),
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
        "--instance",
        "5",
        "--open",
    ]))
    .expect("parses") else {
        panic!("expected sim");
    };
    assert_eq!(sim.profile, Profile::OracleOnly);
    assert_eq!(sim.host_port, 5000);
    assert_eq!(sim.viewer_port, 9000);
    assert_eq!(sim.instance, 5);
    assert!(sim.open);

    let refusal = parse_args(&args(&["sim", "--profile", "simulaton"]));
    assert!(matches!(refusal, Err(XtaskError::UnknownProfile { .. })));
    let refusal = parse_args(&args(&["sim", "--port", "banana"]));
    assert!(matches!(refusal, Err(XtaskError::Usage { .. })));
    for instance in ["-1", "five", "70000"] {
        let refusal = parse_args(&args(&["sim", "--instance", instance]));
        assert!(
            matches!(refusal, Err(XtaskError::Usage { .. })),
            "{instance}"
        );
    }
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

#[test]
fn fleet_takes_one_vehicle_per_flag_and_documented_defaults() {
    let Command::Fleet(fleet) = parse_args(&args(&[
        "fleet",
        "--vehicle",
        "px4-gz:x500_gimbal",
        "--vehicle",
        "px4-gz:rc_cessna",
    ]))
    .expect("parses") else {
        panic!("expected fleet");
    };
    assert_eq!(fleet.vehicles, ["px4-gz:x500_gimbal", "px4-gz:rc_cessna"]);
    assert_eq!(
        (fleet.host_port, fleet.viewer_port, fleet.instance_base),
        (4433, 8080, 0)
    );
    assert!(!fleet.open, "a fleet opens no tabs by default");
    assert_eq!(
        fleet.site,
        pilotage_world::N51,
        "a fleet stands on N51 by default"
    );
    assert!(USAGE.contains("  fleet --vehicle <backend>[:<model>]"));

    let Command::Fleet(fleet) = parse_args(&args(&[
        "fleet",
        "--vehicle",
        "px4-gz",
        "--port",
        "4533",
        "--viewer-port",
        "8180",
        "--instance-base",
        "5",
        "--open",
        "--site",
        "zurich",
    ]))
    .expect("parses") else {
        panic!("expected fleet");
    };
    assert_eq!(
        (
            fleet.host_port,
            fleet.viewer_port,
            fleet.instance_base,
            fleet.open
        ),
        (4533, 8180, 5, true)
    );
    assert_eq!(fleet.site, crate::site::ZURICH);
    assert!(matches!(
        parse_args(&args(&["fleet", "--vehicle", "px4-gz", "--site", "mars"])),
        Err(XtaskError::Usage { .. })
    ));
}

#[test]
fn fleet_refuses_no_vehicles_unknown_flags_and_ports_that_wrap() {
    for malformed in [
        args(&["fleet"]),
        args(&["fleet", "--vehicle"]),
        args(&["fleet", "--vehicle", "px4", "--lan"]),
        args(&[
            "fleet",
            "--vehicle",
            "px4",
            "--vehicle",
            "px4",
            "--port",
            "65535",
        ]),
        args(&["fleet", "--vehicle", "px4", "--port", "0"]),
    ] {
        assert!(
            matches!(parse_args(&malformed), Err(XtaskError::Usage { .. })),
            "{malformed:?} must be refused"
        );
    }
}

/// One readiness deadline for every simulator and FC stage, on both
/// launch commands: 180 s by default, any whole number of seconds above 0.
#[test]
fn ready_timeout_defaults_to_180_s_and_is_settable_on_sim_and_fleet() {
    let Command::Sim(sim) = parse_args(&args(&["sim"])).expect("parses") else {
        panic!("expected sim");
    };
    assert_eq!(sim.ready_timeout_s, 180);
    let Command::Sim(sim) = parse_args(&args(&["sim", "--ready-timeout", "300"])).expect("parses")
    else {
        panic!("expected sim");
    };
    assert_eq!(sim.ready_timeout_s, 300);
    let fleet = |extra: &[&str]| {
        let mut argv = vec!["fleet", "--vehicle", "px4-gz"];
        argv.extend_from_slice(extra);
        parse_args(&args(&argv))
    };
    let Ok(Command::Fleet(defaults)) = fleet(&[]) else {
        panic!("expected fleet");
    };
    assert_eq!(defaults.ready_timeout_s, 180);
    let Ok(Command::Fleet(set)) = fleet(&["--ready-timeout", "240"]) else {
        panic!("expected fleet");
    };
    assert_eq!(set.ready_timeout_s, 240);
    for bad in ["0", "-5", "1.5", "soon"] {
        assert!(
            matches!(
                parse_args(&args(&["sim", "--ready-timeout", bad])),
                Err(XtaskError::Usage { .. })
            ),
            "{bad}"
        );
        assert!(
            matches!(
                fleet(&["--ready-timeout", bad]),
                Err(XtaskError::Usage { .. })
            ),
            "{bad}"
        );
    }
    assert!(USAGE.contains("--ready-timeout <s>"));
}

#[test]
fn build_jsbsim_takes_no_arguments_and_is_documented() {
    assert_eq!(
        parse_args(&args(&["build-jsbsim"])).expect("parses"),
        Command::BuildJsbsim
    );
    assert!(matches!(
        parse_args(&args(&["build-jsbsim", "--tag", "v1.2.0"])),
        Err(XtaskError::Usage { .. })
    ));
    assert_eq!(USAGE.matches("\n  build-jsbsim\n").count(), 1);
    assert!(USAGE.contains(super::super::jsbsim::JSBSIM_TAG));
}

#[test]
fn mission_defaults_to_the_landmark_fleet_and_parses_its_flags() {
    let Command::Mission(mission) = parse_args(&args(&["mission"])).expect("parses") else {
        panic!("expected mission");
    };
    assert_eq!(
        mission.plan,
        std::path::PathBuf::from(super::DEFAULT_MISSION_PLAN)
    );
    assert_eq!(mission.decision, None);
    assert!(!mission.no_fault);
    assert_eq!(mission.ready_timeout_s, 60);
    let Command::Mission(mission) = parse_args(&args(&[
        "mission",
        "--plan",
        "/tmp/fleet.json",
        "--decision",
        "http://127.0.0.1:8761/v1/systemone",
        "--decision-every",
        "8",
        "--no-fault",
        "--instruction-model",
        "http://127.0.0.1:8761/v1/chat/completions",
    ]))
    .expect("parses") else {
        panic!("expected mission");
    };
    assert_eq!(mission.plan, std::path::PathBuf::from("/tmp/fleet.json"));
    assert_eq!(mission.decision_every_s, Some(8.0));
    assert!(mission.no_fault);
    assert_eq!(
        mission.instruction_model.as_deref(),
        Some("http://127.0.0.1:8761/v1/chat/completions")
    );
    assert!(parse_args(&args(&["mission", "--decision-every", "0"])).is_err());
    assert!(parse_args(&args(&["mission", "--fleet"])).is_err());
    assert_eq!(USAGE.matches("  mission [options]\n").count(), 1);
}
