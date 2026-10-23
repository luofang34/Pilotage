use super::{AdapterKind, CliError, parse_args};

#[test]
fn no_args_defaults_to_ephemeral_port() {
    let parsed = parse_args(&[]).expect("empty args parse");
    assert_eq!(parsed.port, 0);
    assert_eq!(parsed.adapter, AdapterKind::Reference);
    assert_eq!(parsed.bind_ip, std::net::Ipv4Addr::LOCALHOST);
    assert!(parsed.join_token_file.is_none());
}

#[test]
fn explicit_port_parses() {
    let args = vec!["--port".to_owned(), "4433".to_owned()];
    let parsed = parse_args(&args).expect("valid port parses");
    assert_eq!(parsed.port, 4433);
}

#[test]
fn gazebo_adapter_parses() {
    let args = vec!["--adapter".to_owned(), "gazebo".to_owned()];
    let parsed = parse_args(&args).expect("gazebo adapter parses");
    assert_eq!(parsed.adapter, AdapterKind::Gazebo);
}

#[test]
fn reference_adapter_parses() {
    let args = vec!["--adapter".to_owned(), "reference".to_owned()];
    let parsed = parse_args(&args).expect("reference adapter parses");
    assert_eq!(parsed.adapter, AdapterKind::Reference);
}

#[test]
fn aviate_adapter_parses() {
    let args = vec!["--adapter".to_owned(), "aviate".to_owned()];
    let parsed = parse_args(&args).expect("aviate adapter parses");
    assert_eq!(parsed.adapter, AdapterKind::Aviate);
}

#[test]
fn px4_adapter_parses() {
    let args = vec!["--adapter".to_owned(), "px4".to_owned()];
    let parsed = parse_args(&args).expect("PX4 adapter parses");
    assert_eq!(parsed.adapter, AdapterKind::Px4);
}

#[test]
fn ardupilot_adapter_parses() {
    let args = vec!["--adapter".to_owned(), "ardupilot".to_owned()];
    let parsed = parse_args(&args).expect("ArduPilot adapter parses");
    assert_eq!(parsed.adapter, AdapterKind::ArduPilot);
}

#[test]
fn betaflight_adapter_parses() {
    let args = vec!["--adapter".to_owned(), "betaflight".to_owned()];
    let parsed = parse_args(&args).expect("Betaflight adapter parses");
    assert_eq!(parsed.adapter, AdapterKind::Betaflight);
}

#[test]
fn unknown_adapter_is_an_error() {
    let args = vec!["--adapter".to_owned(), "unreal".to_owned()];
    assert_eq!(
        parse_args(&args),
        Err(CliError::InvalidAdapter("unreal".to_owned()))
    );
}

#[test]
fn missing_adapter_value_is_an_error() {
    let args = vec!["--adapter".to_owned()];
    assert_eq!(parse_args(&args), Err(CliError::MissingAdapterValue));
}

#[test]
fn missing_port_value_is_an_error() {
    let args = vec!["--port".to_owned()];
    assert_eq!(parse_args(&args), Err(CliError::MissingPortValue));
}

#[test]
fn non_numeric_port_is_an_error() {
    let args = vec!["--port".to_owned(), "not-a-port".to_owned()];
    assert!(matches!(
        parse_args(&args),
        Err(CliError::InvalidPort { .. })
    ));
}

#[test]
fn unrecognized_argument_is_an_error() {
    let args = vec!["--bogus".to_owned()];
    assert_eq!(
        parse_args(&args),
        Err(CliError::Unrecognized("--bogus".to_owned()))
    );
}

#[test]
fn dji_adapter_parses() {
    let args = vec!["--adapter".to_owned(), "dji".to_owned()];
    let parsed = parse_args(&args);
    assert!(matches!(parsed, Ok(value) if value.adapter == AdapterKind::Dji));
}

#[test]
fn sim_multicopter_adapter_parses() {
    let args = vec!["--adapter".to_owned(), "sim-multicopter".to_owned()];
    let parsed = parse_args(&args);
    assert!(matches!(parsed, Ok(value) if value.adapter == AdapterKind::SimMulticopter));
}

#[test]
fn explicit_remote_network_flags_parse_without_embedding_a_secret() {
    let args = [
        "--bind",
        "0.0.0.0",
        "--join-token-file",
        "/tmp/pilotage-token",
        "--approver-token-file",
        "/tmp/pilotage-approver",
    ]
    .map(str::to_owned);
    let parsed = parse_args(&args).unwrap();
    assert_eq!(parsed.bind_ip, std::net::Ipv4Addr::UNSPECIFIED);
    assert_eq!(parsed.join_token_file, Some("/tmp/pilotage-token".into()));
    assert_eq!(
        parsed.approver_token_file,
        Some("/tmp/pilotage-approver".into())
    );
}

#[test]
fn malformed_network_flags_are_rejected() {
    for flag in ["--bind", "--join-token-file", "--approver-token-file"] {
        assert_eq!(
            parse_args(&[flag.to_owned()]),
            Err(CliError::MissingNetworkValue(flag.to_owned()))
        );
    }
    assert!(matches!(
        parse_args(&["--bind".into(), "not-an-address".into()]),
        Err(CliError::InvalidBind(_))
    ));
}
