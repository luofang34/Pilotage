//! The viewer server over real sockets: media types, no-store, HEAD,
//! directory indexes, query strings, and refusal of anything outside the
//! served tree.

#![allow(clippy::expect_used, clippy::panic)]

use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::Arc;

use super::http::ServedTree;
use super::{ServeViewerArgs, parse_args, serve};

/// A served tree `<tmp>/<tag>/web` with a sibling file outside it, a
/// top-level link into a sibling `source` tree, a nested link out of the
/// tree, and a server on an OS-chosen loopback port.
struct Fixture {
    base: PathBuf,
    port: u16,
}

impl Fixture {
    fn start(tag: &str) -> Self {
        let base = std::env::temp_dir().join(format!("plt_xt_serve_{tag}_{}", std::process::id()));
        let web = base.join("web");
        std::fs::create_dir_all(web.join("sub")).expect("the served tree is created");
        std::fs::write(web.join("index.html"), "<title>viewer</title>").expect("index is written");
        std::fs::write(web.join("main.mjs"), "export {};").expect("module is written");
        std::fs::write(web.join("runtime_bg.wasm"), b"\0asm\x01\0\0\0").expect("wasm is written");
        std::fs::write(web.join("sub/index.html"), "sub").expect("sub index is written");
        std::fs::write(base.join("secret.txt"), "outside").expect("outside file is written");
        std::fs::create_dir_all(base.join("source")).expect("the link source is created");
        std::fs::write(base.join("source/linked.js"), "export {};")
            .expect("linked file is written");
        std::os::unix::fs::symlink(base.join("source"), web.join("linked"))
            .expect("a top-level link is made");
        std::os::unix::fs::symlink(&base, web.join("sub/escape")).expect("a nested link is made");
        let root = Arc::new(ServedTree::resolve_blocking(&web).expect("the served tree resolves"));
        let listener = TcpListener::bind("127.0.0.1:0").expect("a loopback port binds");
        let port = listener.local_addr().expect("the port is readable").port();
        std::thread::spawn(move || serve(&listener, &root));
        Self { base, port }
    }

    /// Sends one raw request and returns (head, body).
    fn request(&self, method: &str, target: &str) -> (String, Vec<u8>) {
        let mut stream =
            std::net::TcpStream::connect(("127.0.0.1", self.port)).expect("the server accepts");
        stream
            .write_all(format!("{method} {target} HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n").as_bytes())
            .expect("the request is written");
        let mut raw = Vec::new();
        stream
            .read_to_end(&mut raw)
            .expect("the response is read to EOF");
        let split = raw
            .windows(4)
            .position(|window| window == b"\r\n\r\n")
            .expect("the response has a head");
        let head = String::from_utf8_lossy(&raw[..split]).to_ascii_lowercase();
        (head, raw[split + 4..].to_vec())
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.base).ok();
    }
}

fn status(head: &str) -> &str {
    head.split_whitespace().nth(1).unwrap_or_default()
}

#[test]
fn wasm_is_served_as_application_wasm_without_caching() {
    let fixture = Fixture::start("wasm");
    let (head, body) = fixture.request("GET", "/runtime_bg.wasm");
    assert_eq!(status(&head), "200", "{head}");
    assert!(
        head.contains("content-type: application/wasm\r\n"),
        "{head}"
    );
    assert!(head.contains("cache-control: no-store"), "{head}");
    assert!(head.contains("content-length: 8"), "{head}");
    assert_eq!(body, b"\0asm\x01\0\0\0");
}

#[test]
fn modules_are_javascript() {
    let fixture = Fixture::start("mjs");
    let (head, _) = fixture.request("GET", "/main.mjs");
    assert!(head.contains("content-type: text/javascript"), "{head}");
}

#[test]
fn head_sends_the_headers_without_a_body() {
    let fixture = Fixture::start("head");
    let (head, body) = fixture.request("HEAD", "/runtime_bg.wasm");
    assert_eq!(status(&head), "200", "{head}");
    assert!(head.contains("content-length: 8"), "{head}");
    assert!(head.contains("cache-control: no-store"), "{head}");
    assert!(body.is_empty(), "HEAD carries no body");
}

#[test]
fn directories_serve_their_index_and_queries_are_ignored() {
    let fixture = Fixture::start("index");
    let (head, body) = fixture.request("GET", "/?host=127.0.0.1&port=4433");
    assert_eq!(status(&head), "200", "{head}");
    assert!(head.contains("content-type: text/html"), "{head}");
    assert_eq!(body, b"<title>viewer</title>");

    let (head, _) = fixture.request("GET", "/index.html?host=127.0.0.1#frag");
    assert_eq!(status(&head), "200", "{head}");
}

#[test]
fn a_directory_without_its_slash_redirects_keeping_the_query() {
    let fixture = Fixture::start("redirect");
    let (head, _) = fixture.request("GET", "/sub?host=1");
    assert_eq!(status(&head), "301", "{head}");
    assert!(head.contains("location: /sub/?host=1"), "{head}");
    assert!(head.contains("cache-control: no-store"), "{head}");
}

#[test]
fn paths_outside_the_tree_are_not_found() {
    let fixture = Fixture::start("traversal");
    for target in [
        "/../secret.txt",
        "/%2e%2e/secret.txt",
        "/sub/../../secret.txt",
        "//../secret.txt",
        "/missing.js",
        "/bad%zzescape",
        "/sub/escape/secret.txt",
    ] {
        let (head, body) = fixture.request("GET", target);
        assert!(
            matches!(status(&head), "404" | "400"),
            "{target} must be refused, got {head}"
        );
        assert!(head.contains("cache-control: no-store"), "{head}");
        assert_ne!(body, b"outside", "{target} leaked a file outside the tree");
    }
}

#[test]
fn files_under_a_top_level_link_are_served() {
    let fixture = Fixture::start("linked");
    let (head, body) = fixture.request("GET", "/linked/linked.js");
    assert_eq!(status(&head), "200", "{head}");
    assert_eq!(body, b"export {};");
}

#[test]
fn other_methods_are_refused() {
    let fixture = Fixture::start("method");
    let (head, _) = fixture.request("POST", "/index.html");
    assert_eq!(status(&head), "405", "{head}");
    assert!(head.contains("allow: get, head"), "{head}");
}

#[test]
fn a_connection_closed_without_a_request_does_not_stop_the_server() {
    let fixture = Fixture::start("probe");
    drop(std::net::TcpStream::connect(("127.0.0.1", fixture.port)).expect("the probe connects"));
    let (head, _) = fixture.request("GET", "/");
    assert_eq!(status(&head), "200", "{head}");
}

#[test]
fn arguments_default_to_loopback() {
    let args: Vec<String> = ["--dir", "clients/web", "--port", "8080"]
        .map(str::to_owned)
        .to_vec();
    assert_eq!(
        parse_args(&args).expect("the arguments parse"),
        ServeViewerArgs {
            dir: PathBuf::from("clients/web"),
            port: 8080,
            bind: "127.0.0.1".parse().expect("loopback parses"),
        }
    );
    let missing: Vec<String> = vec!["--dir".to_owned(), "x".to_owned()];
    assert!(parse_args(&missing).is_err(), "--port is required");
}
