//! `xtask serve-viewer` as the session runs it: the built binary serves
//! the viewer's wasm with the media type streaming instantiation needs,
//! and stops on its own once the process that launched it is gone.

#![allow(clippy::expect_used, clippy::panic)]

use std::io::{BufRead, BufReader, Read, Write};
use std::path::PathBuf;
use std::process::{ChildStdout, Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

const EVENT_TIMEOUT: Duration = Duration::from_secs(20);

/// What the watched stdout reported.
#[derive(Debug, PartialEq, Eq)]
enum Event {
    /// A `PID <pid>` line from the launcher shell.
    Pid(u32),
    /// The server's `LISTENING <port>` line.
    Listening(u16),
    /// EOF: every process holding the write end has exited.
    Closed,
}

/// Kills the server by pid if a test fails before it exits on its own,
/// so a failing assertion cannot leak a server.
struct KillOnDrop(u32);

impl Drop for KillOnDrop {
    fn drop(&mut self) {
        Command::new("kill")
            .args(["-KILL", &self.0.to_string()])
            .stderr(Stdio::null())
            .status()
            .ok();
    }
}

fn served_tree(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("plt_xt_sv_{tag}_{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("the served tree is created");
    std::fs::write(dir.join("index.html"), "<title>viewer</title>").expect("index is written");
    std::fs::write(dir.join("runtime_bg.wasm"), b"\0asm\x01\0\0\0").expect("wasm is written");
    dir
}

/// Reads stdout on a thread, reporting `PID` and `LISTENING` lines and
/// finally EOF.
fn watch_stdout(stdout: ChildStdout) -> mpsc::Receiver<Event> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            let line = line.unwrap_or_default();
            if let Some(pid) = line.strip_prefix("PID ") {
                tx.send(Event::Pid(pid.trim().parse().expect("the pid is numeric")))
                    .ok();
            } else if let Some(port) = line.strip_prefix("LISTENING ") {
                tx.send(Event::Listening(
                    port.trim().parse().expect("the port is numeric"),
                ))
                .ok();
            }
        }
        tx.send(Event::Closed).ok();
    });
    rx
}

fn next(events: &mpsc::Receiver<Event>, what: &str) -> Event {
    events
        .recv_timeout(EVENT_TIMEOUT)
        .unwrap_or_else(|_| panic!("timed out waiting for {what}"))
}

fn get(port: u16, target: &str) -> String {
    let mut stream = std::net::TcpStream::connect(("127.0.0.1", port)).expect("the server accepts");
    stream
        .write_all(format!("GET {target} HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n").as_bytes())
        .expect("the request is written");
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).expect("the response is read");
    String::from_utf8_lossy(&raw).to_ascii_lowercase()
}

#[test]
fn the_binary_serves_wasm_without_caching() {
    let dir = served_tree("wasm");
    let mut child = Command::new(env!("CARGO_BIN_EXE_xtask"))
        .args(["serve-viewer", "--port", "0", "--dir"])
        .arg(&dir)
        .stdout(Stdio::piped())
        .spawn()
        .expect("xtask serve-viewer spawns");
    let _guard = KillOnDrop(child.id());
    let events = watch_stdout(child.stdout.take().expect("stdout is piped"));
    let Event::Listening(port) = next(&events, "the LISTENING line") else {
        panic!("the server must report LISTENING first");
    };

    let response = get(port, "/runtime_bg.wasm?host=127.0.0.1");
    child.kill().ok();
    child.wait().ok();
    std::fs::remove_dir_all(&dir).ok();

    let head = response.split("\r\n\r\n").next().unwrap_or_default();
    assert!(head.starts_with("http/1.0 200"), "{head}");
    assert!(head.contains("content-type: application/wasm"), "{head}");
    assert!(head.contains("cache-control: no-store"), "{head}");
}

/// The launcher dying without teardown (SIGKILL, a panic in a test that
/// spawned the stage) must not leave the server holding the port. The
/// shell stays alive until the server is listening, so the server sees
/// it as its parent, then exits; the server must follow it.
#[test]
fn the_server_exits_when_its_launcher_is_gone() {
    let dir = served_tree("orphan");
    let mut launcher = Command::new("sh")
        .arg("-c")
        .arg("\"$0\" serve-viewer --port 0 --dir \"$1\" </dev/null & echo PID $!; read _; exit 0")
        .arg(env!("CARGO_BIN_EXE_xtask"))
        .arg(&dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("the launcher shell spawns");
    let events = watch_stdout(launcher.stdout.take().expect("stdout is piped"));
    let Event::Pid(pid) = next(&events, "the server pid") else {
        panic!("the launcher must report the server pid first");
    };
    let _guard = KillOnDrop(pid);
    let Event::Listening(port) = next(&events, "the LISTENING line") else {
        panic!("the server must report LISTENING");
    };

    launcher
        .stdin
        .take()
        .expect("stdin is piped")
        .write_all(b"go\n")
        .expect("the launcher is released");
    launcher.wait().expect("the launcher shell exits");
    let exited = events.recv_timeout(EVENT_TIMEOUT);
    std::fs::remove_dir_all(&dir).ok();

    assert_eq!(
        exited,
        Ok(Event::Closed),
        "the orphaned server on port {port} must exit once its launcher is gone"
    );
    assert!(
        std::net::TcpStream::connect(("127.0.0.1", port)).is_err(),
        "nothing listens on {port} after the server exited"
    );
}
