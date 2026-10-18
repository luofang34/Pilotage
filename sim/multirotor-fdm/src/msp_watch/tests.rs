//! The watch against fake MSP peers on loopback: one that never answers
//! is a hang after the silence limit, one that answers never is.

#![allow(clippy::expect_used, clippy::panic)]

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use super::{MspWatch, REQUEST_PERIOD, SILENCE_LIMIT};

#[test]
fn a_peer_that_accepts_and_never_answers_is_a_hang() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let mut watch = MspWatch::new(listener.local_addr().expect("addr"));
    let start = Instant::now();
    assert!(!watch.poll(start), "connected, not yet silent");
    let (_peer, _) = listener.accept().expect("the watch connects");
    let just_short = start + SILENCE_LIMIT - Duration::from_millis(1);
    assert!(!watch.poll(just_short));
    assert!(watch.poll(start + SILENCE_LIMIT), "silent for the limit");
}

#[test]
fn a_peer_that_answers_is_never_a_hang() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let mut watch = MspWatch::new(listener.local_addr().expect("addr"));
    let (answered, replies) = mpsc::channel();
    let start = Instant::now();
    assert!(!watch.poll(start));
    let (mut peer, _) = listener.accept().expect("the watch connects");
    std::thread::spawn(move || {
        let mut request = [0_u8; 6];
        while peer.read_exact(&mut request).is_ok() {
            // MSP_API_VERSION: protocol 0, API 1.49.
            if peer.write_all(b"$M>\x03\x01\x00\x01\x31\x32").is_err() {
                return;
            }
            if answered.send(()).is_err() {
                return;
            }
        }
    });
    let mut now = start;
    for _ in 0..30 {
        replies.recv().expect("the peer answered");
        now += REQUEST_PERIOD;
        assert!(!watch.poll(now), "an answering peer is not a hang");
    }
    assert!(now - start >= SILENCE_LIMIT * 2);
}

#[test]
fn a_refused_port_is_not_a_hang() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("addr");
    drop(listener);
    let mut watch = MspWatch::new(addr);
    let start = Instant::now();
    assert!(!watch.poll(start));
    assert!(!watch.poll(start + SILENCE_LIMIT * 2));
}
