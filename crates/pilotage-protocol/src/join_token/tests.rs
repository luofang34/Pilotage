use super::*;

#[test]
fn token_decoding_matching_and_debug_preserve_the_secret_boundary() {
    let value = "0123456789abcdef".repeat(4);
    let token = JoinToken::from_hex(&value).unwrap();
    assert_eq!(token, JoinToken::from_hex(&value.to_uppercase()).unwrap());
    assert!(token.matches(token.as_bytes()));
    assert!(!token.matches(&[]));
    assert!(!token.matches(&[0; 32]));
    for index in 0..32 {
        let mut changed = *token.as_bytes();
        changed[index] ^= 1;
        assert!(!token.matches(&changed));
    }
    assert_eq!(format!("{token:?}"), "JoinToken([redacted])");
    assert!(!format!("{token:?}").contains(&value));
}

#[test]
fn invalid_tokens_do_not_echo_the_input() {
    for value in ["", "bad secret", &"a".repeat(63), &"a".repeat(65)] {
        assert_eq!(
            JoinToken::from_hex(value),
            Err(JoinTokenError::InvalidLength)
        );
    }
    let invalid = format!("{}?", "a".repeat(63));
    let error = JoinToken::from_hex(&invalid).unwrap_err();
    assert_eq!(error, JoinTokenError::InvalidHex);
    assert!(!format!("{error} {error:?}").contains(&invalid));
}

#[test]
fn hello_debug_never_includes_secret_bytes() {
    let secret = vec![171; 32];
    let hello = crate::ClientHello {
        protocol_version: crate::SESSION_PROTOCOL_VERSION,
        client_name: "test".into(),
        join_token: secret.clone(),
    };
    let debug = format!("{hello:?}");
    assert!(debug.contains("[redacted]"));
    assert!(!debug.contains(&format!("{secret:?}")));
}
