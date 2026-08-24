#![allow(clippy::expect_used, clippy::panic)]

use crate::{DurableStore, StorageError};

#[test]
fn non_unix_store_refuses_a_weaker_contract() {
    let path = std::path::Path::new("storage");
    let error = DurableStore::open_or_create(path).expect_err("reject unsupported storage");
    assert!(matches!(error, StorageError::UnsupportedPlatform { .. }));
    assert_eq!(error.context().requested_root.as_deref(), Some(path));
}
