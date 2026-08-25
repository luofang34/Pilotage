#![allow(clippy::expect_used, clippy::panic)]

use std::process::{Command, Stdio};

use super::cleanup_unconfigured_gate;
use crate::AviateSupervisorError;

#[test]
fn cleanup_accepts_an_already_reaped_gate() {
    let mut child = Command::new(std::env::current_exe().expect("find test executable"))
        .arg("--help")
        .env_clear()
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn short test process");
    child.wait().expect("reap short test process");
    let result = cleanup_unconfigured_gate::<()>(
        child,
        AviateSupervisorError::protocol("test startup failure"),
    );

    assert!(matches!(
        result,
        Err(AviateSupervisorError::Protocol { detail, .. }) if detail == "test startup failure"
    ));
}
