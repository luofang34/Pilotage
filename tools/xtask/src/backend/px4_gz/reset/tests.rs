//! A reset reaches only its own instance, and only a supervised one.

#![allow(clippy::expect_used, clippy::panic)]

use super::{instance_of, reset_instance};
use crate::error::XtaskError;

#[test]
fn the_instance_is_read_from_the_px4_command_line() {
    let px4 = "/p/build/px4_sitl_default/bin/px4 /p/build/px4_sitl_default/etc -s etc/init.d-posix/rcS -d";
    assert_eq!(instance_of(px4), 0, "no -i is PX4's instance 0");
    assert_eq!(instance_of(&format!("{px4} -i 7")), 7);
    assert_eq!(instance_of(&format!("{px4} -i 12")), 12);
    assert_ne!(instance_of(&format!("{px4} -i 17")), 1);
}

/// With no live supervisor nothing would start px4 again, so the reset
/// refuses before it stops anything.
#[test]
fn an_unsupervised_instance_is_refused_before_anything_stops() {
    let repo = std::env::temp_dir().join(format!("plt_px4_reset_{}", std::process::id()));
    std::fs::create_dir_all(&repo).expect("repo");
    let refusal = reset_instance(&repo, &repo.join("PX4-Autopilot"), "default", 7);
    assert!(matches!(
        refusal,
        Err(XtaskError::Unsupervised { instance: 7 })
    ));

    // A marker naming a process that has exited is not a supervisor.
    let marker = crate::session::supervisor_marker(&repo, 7);
    std::fs::create_dir_all(marker.parent().expect("dir")).expect("dir");
    let mut exited = std::process::Command::new("true").spawn().expect("spawn");
    let pid = exited.id();
    exited.wait().expect("exits");
    std::fs::write(&marker, pid.to_string()).expect("marker");
    let refusal = reset_instance(&repo, &repo.join("PX4-Autopilot"), "default", 7);
    assert!(matches!(
        refusal,
        Err(XtaskError::Unsupervised { instance: 7 })
    ));
    std::fs::remove_dir_all(&repo).ok();
}
