#![allow(clippy::expect_used, clippy::panic)]

use super::{CONFIG_BLOB_MAX, ConfigBlob, ConfigError, ConfigKey, keys};

fn entry(key: u16, payload: &[u8]) -> std::vec::Vec<u8> {
    let mut bytes = std::vec::Vec::new();
    bytes.extend_from_slice(&key.to_le_bytes());
    bytes.extend_from_slice(&(payload.len() as u16).to_le_bytes());
    bytes.extend_from_slice(payload);
    bytes
}

#[test]
fn an_empty_blob_parses_and_yields_nothing() {
    let blob = ConfigBlob::parse(&[]).expect("empty is the default config");
    assert_eq!(blob.get(keys::BACKGROUND_MODE), None);
    assert_eq!(blob.require_schema(&[]), Ok(()));
}

#[test]
fn entries_are_retrievable_by_key() {
    let mut bytes = entry(keys::BACKGROUND_MODE.0, &[1]);
    bytes.extend_from_slice(&entry(keys::V_SPEEDS.0, &[0; 20]));
    let blob = ConfigBlob::parse(&bytes).expect("two well-formed entries");
    assert_eq!(blob.get(keys::BACKGROUND_MODE), Some(&[1u8][..]));
    assert_eq!(blob.get(keys::V_SPEEDS).map(<[u8]>::len), Some(20));
    assert_eq!(blob.get(keys::SVS_QUALITY), None);
}

#[test]
fn an_oversize_blob_is_refused() {
    let bytes = [0u8; CONFIG_BLOB_MAX + 1];
    assert_eq!(
        ConfigBlob::parse(&bytes).map(|_| ()),
        Err(ConfigError::TooLong {
            len: CONFIG_BLOB_MAX + 1
        })
    );
}

#[test]
fn truncation_is_refused_wherever_it_falls() {
    // Inside an entry header.
    assert_eq!(
        ConfigBlob::parse(&[0x01, 0x00, 0x05]).map(|_| ()),
        Err(ConfigError::Truncated { key: 1 })
    );
    // Payload runs past the end.
    let mut bytes = entry(keys::BACKGROUND_MODE.0, &[1]);
    bytes.pop();
    assert_eq!(
        ConfigBlob::parse(&bytes).map(|_| ()),
        Err(ConfigError::Truncated {
            key: keys::BACKGROUND_MODE.0
        })
    );
}

#[test]
fn descending_or_repeated_keys_are_refused() {
    let mut descending = entry(keys::V_SPEEDS.0, &[0; 20]);
    descending.extend_from_slice(&entry(keys::BACKGROUND_MODE.0, &[0]));
    assert_eq!(
        ConfigBlob::parse(&descending).map(|_| ()),
        Err(ConfigError::KeysNotAscending {
            key: keys::BACKGROUND_MODE.0
        })
    );
    let mut repeated = entry(keys::BACKGROUND_MODE.0, &[0]);
    repeated.extend_from_slice(&entry(keys::BACKGROUND_MODE.0, &[1]));
    assert_eq!(
        ConfigBlob::parse(&repeated).map(|_| ()),
        Err(ConfigError::KeysNotAscending {
            key: keys::BACKGROUND_MODE.0
        })
    );
}

#[test]
fn a_key_outside_the_schema_is_rejected_not_skipped() {
    let bytes = entry(0x8000, &[7]);
    let blob = ConfigBlob::parse(&bytes).expect("well-formed foreign entry");
    assert_eq!(
        blob.require_schema(&[keys::BACKGROUND_MODE, keys::V_SPEEDS]),
        Err(ConfigError::UnknownKey { key: 0x8000 })
    );
    assert_eq!(blob.require_schema(&[ConfigKey(0x8000)]), Ok(()));
}
