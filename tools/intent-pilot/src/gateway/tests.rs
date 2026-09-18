#![allow(clippy::expect_used, clippy::panic)]

use std::net::SocketAddr;
use std::sync::Arc;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Notify;
use tokio::task::JoinHandle;

use super::{GatewayConfig, GatewayError, serve};
use crate::model_process::ModelProcess;

const PAGE: &str = "http://localhost:8099";

/// A model adapter in shell: the declaration, then the same directive for each
/// request line, or a fault line for a request that says `fault`.
const ADAPTER: &str = r#"printf '%s\n' '{"ready":true,"adapter":"shell/1","model":"fixed","kinds":["land"]}'
while IFS= read -r line; do
  id=$(printf '%s' "$line" | sed -n 's/.*"id":\([0-9]*\).*/\1/p')
  case "$line" in
    *fault*) printf '{"id":%s,"error":"the model is not loaded"}\n' "$id" ;;
    *) printf '{"id":%s,"directive":{"kind":"land"},"probabilities":{},"model_ms":1.0}\n' "$id" ;;
  esac
done"#;

const REQUEST_BODY: &str = r#"{"message":"Cleared to land.","envelope":{"kinds":["land"],"fixes":["HOME"],"procedures":[],"height_m":{"min":2.0,"max":30.0},"speed_mps":{"min":0.3,"max":5.0}},"legend":"FIXES: HOME is the launch point and base.","frames":[]}"#;

struct Gateway {
    address: SocketAddr,
    shutdown: Arc<Notify>,
    task: JoinHandle<Result<(), GatewayError>>,
}

impl Gateway {
    async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let address = listener.local_addr().expect("address");
        let model = ModelProcess::spawn(ADAPTER)
            .await
            .expect("the shell adapter starts");
        let shutdown = Arc::new(Notify::new());
        let config = GatewayConfig {
            allowed_origins: vec![PAGE.to_owned()],
        };
        let task = tokio::spawn(serve(listener, model, config, Arc::clone(&shutdown)));
        Self {
            address,
            shutdown,
            task,
        }
    }

    /// Sends raw request bytes and reads the complete answer.
    async fn exchange(&self, request: &str) -> String {
        let mut stream = TcpStream::connect(self.address).await.expect("connect");
        stream.write_all(request.as_bytes()).await.expect("write");
        let mut answer = String::new();
        stream.read_to_string(&mut answer).await.expect("read");
        answer
    }

    async fn stop(self) {
        self.shutdown.notify_one();
        self.task
            .await
            .expect("join")
            .expect("the gateway ends cleanly");
    }
}

fn post(body: &str, origin: Option<&str>) -> String {
    let origin = origin.map_or(String::new(), |origin| format!("Origin: {origin}\r\n"));
    format!(
        "POST /v1/directive HTTP/1.1\r\nHost: gateway\r\n{origin}Content-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    )
}

fn body_of(answer: &str) -> &str {
    answer.split_once("\r\n\r\n").map_or("", |(_, body)| body)
}

#[tokio::test]
async fn a_request_line_gets_the_reply_line_of_the_adapter() {
    let gateway = Gateway::start().await;
    let answer = gateway.exchange(&post(REQUEST_BODY, Some(PAGE))).await;
    assert!(answer.starts_with("HTTP/1.1 200 OK\r\n"), "{answer}");
    assert!(
        answer.contains(&format!("Access-Control-Allow-Origin: {PAGE}\r\n")),
        "{answer}"
    );
    let reply: serde_json::Value = serde_json::from_str(body_of(&answer)).expect("a JSON body");
    assert_eq!(reply["directive"]["kind"], "land");
    gateway.stop().await;
}

#[tokio::test]
async fn the_declaration_is_the_declaration_of_the_adapter() {
    let gateway = Gateway::start().await;
    let answer = gateway
        .exchange("GET /v1/declaration HTTP/1.1\r\nHost: gateway\r\n\r\n")
        .await;
    let declared: serde_json::Value = serde_json::from_str(body_of(&answer)).expect("JSON");
    assert_eq!(declared["adapter"], "shell/1");
    assert_eq!(declared["model"], "fixed");
    gateway.stop().await;
}

#[tokio::test]
async fn an_adapter_fault_is_a_fault_line_with_status_200() {
    let gateway = Gateway::start().await;
    let body = REQUEST_BODY.replace("Cleared to land.", "fault");
    let answer = gateway.exchange(&post(&body, None)).await;
    assert!(answer.starts_with("HTTP/1.1 200 OK\r\n"), "{answer}");
    let fault: serde_json::Value = serde_json::from_str(body_of(&answer)).expect("JSON");
    assert!(
        fault["error"]
            .as_str()
            .is_some_and(|detail| detail.contains("the model is not loaded")),
        "{fault}"
    );
    // The adapter still answers after a fault.
    let next = gateway.exchange(&post(REQUEST_BODY, None)).await;
    assert!(body_of(&next).contains("\"land\""), "{next}");
    gateway.stop().await;
}

#[tokio::test]
async fn a_page_that_is_not_permitted_is_refused_and_gets_no_grant() {
    let gateway = Gateway::start().await;
    let answer = gateway
        .exchange(&post(REQUEST_BODY, Some("http://other.example")))
        .await;
    assert!(answer.starts_with("HTTP/1.1 403 Forbidden\r\n"), "{answer}");
    assert!(!answer.contains("Access-Control-Allow-Origin"), "{answer}");
    gateway.stop().await;
}

#[tokio::test]
async fn a_preflight_of_a_permitted_page_gets_the_methods_and_the_header() {
    let gateway = Gateway::start().await;
    let answer = gateway
        .exchange(&format!(
            "OPTIONS /v1/directive HTTP/1.1\r\nHost: gateway\r\nOrigin: {PAGE}\r\nAccess-Control-Request-Method: POST\r\n\r\n"
        ))
        .await;
    assert!(
        answer.starts_with("HTTP/1.1 204 No Content\r\n"),
        "{answer}"
    );
    assert!(answer.contains("Access-Control-Allow-Methods: GET, POST, OPTIONS\r\n"));
    assert!(answer.contains("Access-Control-Allow-Headers: Content-Type\r\n"));
    gateway.stop().await;
}

#[tokio::test]
async fn a_body_that_is_not_a_model_request_and_an_unknown_path_are_refused() {
    let gateway = Gateway::start().await;
    let answer = gateway.exchange(&post("{\"message\":1}", None)).await;
    assert!(
        answer.starts_with("HTTP/1.1 400 Bad Request\r\n"),
        "{answer}"
    );
    let answer = gateway
        .exchange("GET /v1/other HTTP/1.1\r\nHost: gateway\r\n\r\n")
        .await;
    assert!(answer.starts_with("HTTP/1.1 404 Not Found\r\n"), "{answer}");
    let answer = gateway.exchange("not http\r\n\r\n").await;
    assert!(
        answer.starts_with("HTTP/1.1 400 Bad Request\r\n"),
        "{answer}"
    );
    gateway.stop().await;
}

#[tokio::test]
async fn a_body_in_two_writes_is_read_whole() {
    let gateway = Gateway::start().await;
    let request = post(REQUEST_BODY, None);
    let (head, tail) = request.split_at(request.len() - 40);
    let mut stream = TcpStream::connect(gateway.address).await.expect("connect");
    stream.write_all(head.as_bytes()).await.expect("write head");
    stream.flush().await.expect("flush");
    stream.write_all(tail.as_bytes()).await.expect("write tail");
    let mut answer = String::new();
    stream.read_to_string(&mut answer).await.expect("read");
    assert!(body_of(&answer).contains("\"land\""), "{answer}");
    gateway.stop().await;
}
