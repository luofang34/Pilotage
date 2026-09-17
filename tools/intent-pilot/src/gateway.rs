//! The model gateway: the model port over HTTP, for a client that cannot
//! start a process.
//!
//! A browser cannot run a model adapter. The gateway holds the adapter process
//! and gives the same exchange over HTTP: one request line in, one reply line
//! out. It adds no second protocol. The body of `POST /v1/directive` is a
//! [`ModelRequest`], and the answer is a [`ModelReply`] or a fault line.
//! `GET /v1/declaration` gives the declaration of the adapter.
//!
//! The gateway holds no authority. It sends nothing to a vehicle and reads
//! nothing from one.

mod http;

use std::sync::Arc;
use std::time::Duration;

use pilotage_agent::{AdapterDeclaration, ModelReply, ModelRequest};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{Notify, mpsc, oneshot};

use crate::model_process::ModelProcess;
use http::{Request, Response, read_request, write_response};

/// Time allowed for one client to send its request.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
/// Requests that can wait for the single adapter process.
const ASK_QUEUE: usize = 16;

/// One question for the adapter, with the place for its answer.
type Ask = (ModelRequest, oneshot::Sender<Result<ModelReply, String>>);

/// Why the gateway cannot serve.
#[derive(Debug, thiserror::Error)]
pub enum GatewayError {
    /// The listener did not accept a connection.
    #[error("the gateway listener failed")]
    Accept(#[source] std::io::Error),
}

/// What the gateway serves and to which pages.
#[derive(Debug, Clone)]
pub struct GatewayConfig {
    /// The page origins that a browser can call from. A request with a
    /// different `Origin` header is refused.
    pub allowed_origins: Vec<String>,
}

/// The handle that each connection uses to reach the one adapter process.
#[derive(Debug, Clone)]
struct ModelHandle {
    declaration: Arc<AdapterDeclaration>,
    asks: mpsc::Sender<Ask>,
}

/// Serves the model port on `listener` until `shutdown` is notified.
pub async fn serve(
    listener: TcpListener,
    model: ModelProcess,
    config: GatewayConfig,
    shutdown: Arc<Notify>,
) -> Result<(), GatewayError> {
    let (asks, queue) = mpsc::channel(ASK_QUEUE);
    let handle = ModelHandle {
        declaration: Arc::new(model.declaration().clone()),
        asks,
    };
    let model_task = tokio::spawn(run_model(model, queue));
    let config = Arc::new(config);
    let result = loop {
        tokio::select! {
            () = shutdown.notified() => break Ok(()),
            accepted = listener.accept() => match accepted {
                Ok((stream, peer)) => {
                    tracing::debug!(%peer, "gateway connection");
                    tokio::spawn(connection(stream, handle.clone(), Arc::clone(&config)));
                }
                Err(source) => break Err(GatewayError::Accept(source)),
            },
        }
    };
    drop(handle);
    if let Err(error) = model_task.await {
        tracing::warn!(%error, "the model task did not end cleanly");
    }
    result
}

/// Owns the adapter process. One request is at the adapter at a time, because
/// the process port is one line in and one line out.
async fn run_model(mut model: ModelProcess, mut queue: mpsc::Receiver<Ask>) {
    while let Some((request, answer)) = queue.recv().await {
        let result = model
            .ask(&request)
            .await
            .map(|(reply, _)| reply)
            .map_err(|error| describe(&error));
        if let Err(detail) = &result {
            tracing::error!(%detail, "model fault");
        }
        // A client that went away has no use for the answer.
        answer.send(result).ok();
    }
    model.stop().await;
}

async fn connection(mut stream: TcpStream, model: ModelHandle, config: Arc<GatewayConfig>) {
    let request = match tokio::time::timeout(REQUEST_TIMEOUT, read_request(&mut stream)).await {
        Ok(Ok(request)) => request,
        Ok(Err(error)) => {
            tracing::warn!(%error, "the request is not usable");
            let response = Response::fault(400, &error.to_string());
            write_response(&mut stream, &response, None).await.ok();
            return;
        }
        Err(_) => return,
    };
    let origin = match allowed_origin(&request, &config) {
        Ok(origin) => origin,
        Err(response) => {
            write_response(&mut stream, &response, None).await.ok();
            return;
        }
    };
    let response = route(&request, &model).await;
    if let Err(error) = write_response(&mut stream, &response, origin.as_deref()).await {
        tracing::debug!(%error, "the client closed before the response");
    }
}

/// The origin to echo, or the refusal for a page that is not permitted. A
/// request with no `Origin` header is not from a page, and needs no grant.
fn allowed_origin(request: &Request, config: &GatewayConfig) -> Result<Option<String>, Response> {
    match &request.origin {
        None => Ok(None),
        Some(origin)
            if config
                .allowed_origins
                .iter()
                .any(|allowed| allowed == origin) =>
        {
            Ok(Some(origin.clone()))
        }
        Some(origin) => Err(Response::fault(
            403,
            &format!("the origin {origin} is not permitted"),
        )),
    }
}

async fn route(request: &Request, model: &ModelHandle) -> Response {
    match (request.method.as_str(), request.path.as_str()) {
        ("OPTIONS", _) => Response::preflight(),
        ("GET", "/v1/declaration") => Response::json(200, model.declaration.as_ref()),
        ("POST", "/v1/directive") => directive(&request.body, model).await,
        _ => Response::fault(404, "the gateway has /v1/declaration and /v1/directive"),
    }
}

async fn directive(body: &[u8], model: &ModelHandle) -> Response {
    let request: ModelRequest = match serde_json::from_slice(body) {
        Ok(request) => request,
        Err(error) => {
            return Response::fault(400, &format!("the body is not a model request: {error}"));
        }
    };
    let (answer, reply) = oneshot::channel();
    if model.asks.send((request, answer)).await.is_err() {
        return Response::fault(200, "the model adapter stopped");
    }
    // A model fault is a valid answer of the model port, so it goes out as a
    // fault line with status 200. The agent core classifies it.
    match reply.await {
        Ok(Ok(reply)) => Response::json(200, &reply),
        Ok(Err(detail)) => Response::fault(200, &detail),
        Err(_) => Response::fault(200, "the model adapter stopped"),
    }
}

/// An error with each cause, in one line.
fn describe(error: &dyn std::error::Error) -> String {
    let mut text = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        text.push_str(": ");
        text.push_str(&cause.to_string());
        source = cause.source();
    }
    text
}

#[cfg(test)]
mod tests;
