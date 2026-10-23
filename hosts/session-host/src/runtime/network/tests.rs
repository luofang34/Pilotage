use super::*;

#[test]
fn remote_interfaces_require_a_token_but_local_defaults_are_usable() {
    assert_eq!(
        NetworkOptions::default().bind_ip,
        IpAddr::V4(Ipv4Addr::LOCALHOST)
    );
    assert!(NetworkOptions::default().validate().is_ok());
    for address in ["0.0.0.0", "::", "192.0.2.2"] {
        let mut options = NetworkOptions {
            bind_ip: address.parse().unwrap(),
            join_token: None,
            approver_token: None,
        };
        assert!(matches!(
            options.validate(),
            Err(NetworkError::RemoteTokenRequired { .. })
        ));
        options.join_token = Some(JoinToken::from_hex(&"ab".repeat(32)).unwrap());
        assert!(options.validate().is_ok());
    }
}

#[test]
fn a_relative_token_file_is_refused_before_reading() {
    assert!(matches!(
        NetworkOptions::from_token_file_blocking(
            Ipv4Addr::LOCALHOST.into(),
            Some(Path::new("token"))
        ),
        Err(NetworkError::RelativeTokenPath { .. })
    ));
}

#[test]
fn token_file_accepts_surrounding_whitespace_and_redacts_loaded_options() {
    let secret = "aB".repeat(32);
    let file = TokenFile::new("valid", format!(" \n{secret}\t\r\n").as_bytes());
    let options =
        NetworkOptions::from_token_file_blocking(Ipv4Addr::UNSPECIFIED.into(), Some(&file.0))
            .unwrap();
    assert_eq!(
        options.join_token,
        Some(JoinToken::from_hex(&secret).unwrap())
    );
    assert!(!format!("{options:?}").contains(&secret));
}

#[test]
fn invalid_token_file_reports_its_path_without_contents() {
    let secret = format!("{}?", "a".repeat(63));
    let file = TokenFile::new("invalid", secret.as_bytes());
    let error = NetworkOptions::from_token_file_blocking(Ipv4Addr::LOCALHOST.into(), Some(&file.0))
        .unwrap_err();
    assert!(matches!(
        error,
        NetworkError::TokenFormat {
            source: JoinTokenError::InvalidHex,
            ..
        }
    ));
    assert!(error.to_string().contains(file.0.to_str().unwrap()));
    assert!(!format!("{error} {error:?}").contains(&secret));
}

#[test]
fn oversized_token_file_is_bounded_and_missing_file_preserves_io_error() {
    let file = TokenFile::new("oversized", &vec![b'a'; 4096]);
    assert!(matches!(
        NetworkOptions::from_token_file_blocking(Ipv4Addr::LOCALHOST.into(), Some(&file.0)),
        Err(NetworkError::TokenFormat {
            source: JoinTokenError::InvalidLength,
            ..
        })
    ));
    std::fs::remove_file(&file.0).unwrap();
    let error = NetworkOptions::from_token_file_blocking(Ipv4Addr::LOCALHOST.into(), Some(&file.0))
        .unwrap_err();
    assert!(matches!(
        error,
        NetworkError::TokenRead { source, path }
            if source.kind() == std::io::ErrorKind::NotFound && path == file.0
    ));
}

struct TokenFile(PathBuf);

impl TokenFile {
    fn new(label: &str, contents: &[u8]) -> Self {
        use std::io::Write;
        let path = std::env::temp_dir().join(format!(
            "pilotage-admission-{}-{label}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::File::create_new(&path)
            .unwrap()
            .write_all(contents)
            .unwrap();
        Self(path)
    }
}

impl Drop for TokenFile {
    fn drop(&mut self) {
        std::fs::remove_file(&self.0).ok();
    }
}

#[test]
fn the_approver_secret_admits_as_approver_and_the_join_secret_does_not() {
    let join = JoinToken::from_hex(&"ab".repeat(32)).unwrap();
    let approver = JoinToken::from_hex(&"cd".repeat(32)).unwrap();
    let open = super::Admission::default();
    assert_eq!(
        open.check(&[]),
        Some(true),
        "without secrets everyone approves"
    );

    let guarded = super::Admission {
        join_token: Some(join.clone()),
        approver_token: Some(approver.clone()),
    };
    assert_eq!(guarded.check(approver.as_bytes()), Some(true));
    assert_eq!(guarded.check(join.as_bytes()), Some(false));
    assert_eq!(guarded.check(&[]), None);
    assert_eq!(guarded.check(&[0xee; 32]), None);

    let approver_only = super::Admission {
        join_token: None,
        approver_token: Some(approver.clone()),
    };
    assert_eq!(approver_only.check(&[]), Some(false));
    assert_eq!(approver_only.check(approver.as_bytes()), Some(true));
}
