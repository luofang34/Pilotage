//! Guard tests against real sockets and processes: a claim is refused
//! exactly when something else holds it.

#![allow(clippy::expect_used, clippy::panic)]

use std::net::{TcpListener, UdpSocket};

use super::{
    Claim, Remedy, first_world_topic, held_by_another, refuse_conflicts, tcp_port_in_use,
    udp_port_in_use,
};
use crate::error::XtaskError;

#[test]
fn a_udp_port_held_on_loopback_or_every_interface_is_in_use() {
    for bind in ["127.0.0.1:0", "0.0.0.0:0"] {
        let held = UdpSocket::bind(bind).expect("bind a free port");
        let port = held.local_addr().expect("local addr").port();
        assert!(udp_port_in_use(port), "{bind} holder must be seen");
    }
}

#[test]
fn a_tcp_port_listened_on_loopback_or_every_interface_is_in_use() {
    for bind in ["127.0.0.1:0", "0.0.0.0:0"] {
        let held = TcpListener::bind(bind).expect("bind a free port");
        let port = held.local_addr().expect("local addr").port();
        assert!(tcp_port_in_use(port), "{bind} listener must be seen");
    }
}

#[test]
fn only_held_claims_are_refused_and_every_one_is_named() {
    let udp = UdpSocket::bind("127.0.0.1:0").expect("bind");
    let udp_port = udp.local_addr().expect("addr").port();
    let tcp = TcpListener::bind("127.0.0.1:0").expect("bind");
    let tcp_port = tcp.local_addr().expect("addr").port();
    // Not a released port: on a shared machine another process may take
    // it before the probe runs. A pattern nothing matches is never held.
    let unheld = Claim::Process("plt-claim-pattern-no-process-has");

    let claims = [
        Claim::Udp {
            port: udp_port,
            holder: "session host".to_owned(),
            remedy: Remedy::Instance,
        },
        unheld,
        Claim::Tcp {
            port: tcp_port,
            holder: "viewer".to_owned(),
            remedy: Remedy::Instance,
        },
    ];
    let Err(XtaskError::SessionConflict { conflicts }) = refuse_conflicts(&claims) else {
        panic!("held ports must be refused");
    };
    assert_eq!(conflicts.len(), 2, "exactly the held claims: {conflicts:?}");
    assert!(conflicts[0].contains(&udp_port.to_string()));
    assert!(conflicts[1].contains(&tcp_port.to_string()));

    refuse_conflicts(&claims[1..2]).expect("a free claim starts");
}

#[test]
fn a_process_claim_is_held_only_while_a_matching_process_runs() {
    let marker = format!("plt-claim-{}", std::process::id());
    let pattern: &'static str = Box::leak(marker.clone().into_boxed_str());
    assert!(held_by_another(&Claim::Process(pattern)).is_none());

    let mut child = std::process::Command::new("sh")
        .args(["-c", "read line", &marker])
        .stdin(std::process::Stdio::piped())
        .spawn()
        .expect("spawn marker process");
    let held = held_by_another(&Claim::Process(pattern)).expect("the marker process holds it");
    assert!(
        held.contains(&child.id().to_string()),
        "the holder is named: {held}"
    );
    drop(child.stdin.take());
    child.wait().expect("marker exits on EOF");
    assert!(held_by_another(&Claim::Process(pattern)).is_none());
}

#[test]
fn a_world_topic_marks_the_partition_as_used() {
    assert_eq!(first_world_topic(""), None);
    assert_eq!(first_world_topic("/gazebo/resource_paths\n/stats\n"), None);
    assert_eq!(
        first_world_topic("/clock\n/world/default/clock\n/world/default/stats\n"),
        Some("/world/default/clock".to_owned())
    );
}

/// The refusal says what moves each held claim, and suggests another
/// instance only for a claim that moves with the instance.
#[test]
fn each_refusal_names_what_frees_that_claim() {
    let held = UdpSocket::bind("127.0.0.1:0").expect("bind");
    let port = held.local_addr().expect("addr").port();
    let message = |remedy| {
        held_by_another(&Claim::Udp {
            port,
            holder: "test".to_owned(),
            remedy,
        })
        .expect("held")
    };
    assert!(message(Remedy::Flag("--port")).ends_with("pick another --port"));
    assert!(message(Remedy::Instance).contains("another --instance"));
    let fixed = message(Remedy::StopOther);
    assert!(fixed.ends_with("stop the session that holds it"));
    assert!(!fixed.contains("--instance"), "{fixed}");
}

#[test]
fn a_session_names_the_flag_that_moves_its_host_and_viewer_ports() {
    use super::session_claims;
    use crate::backend::SessionContext;

    struct Bare;
    impl crate::backend::SimBackend for Bare {
        fn name(&self) -> &'static str {
            "bare"
        }
        fn host_adapter(&self) -> &'static str {
            "reference"
        }
        fn host_env(&self, _ctx: &SessionContext) -> Vec<(String, String)> {
            Vec::new()
        }
        fn plan(&self, _ctx: &SessionContext) -> Result<Vec<crate::backend::Stage>, XtaskError> {
            Ok(Vec::new())
        }
        fn stale_process_patterns(&self) -> Vec<&'static str> {
            vec!["bare-singleton"]
        }
        fn reset(&self, _repo_root: &std::path::Path) -> Result<(), XtaskError> {
            Ok(())
        }
    }
    let claims = session_claims(&SessionContext::for_test("unused".into()), &Bare);
    assert!(matches!(
        claims[0],
        Claim::Udp {
            remedy: Remedy::Flag("--port"),
            ..
        }
    ));
    assert!(matches!(
        claims[1],
        Claim::Tcp {
            remedy: Remedy::Flag("--viewer-port"),
            ..
        }
    ));
    assert_eq!(claims[2], Claim::Process("bare-singleton"));
}
