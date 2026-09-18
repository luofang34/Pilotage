//! The WebTransport port: sockets and tasks, no protocol decisions.
//!
//! Reader tasks report bytes as `TransportEvent`s. The client engine in
//! the pilot loop decides what the bytes mean. The port pins the host's
//! certificate hash, as the browser client does, and has no option that
//! turns certificate validation off.

use std::sync::Arc;
use std::time::Duration;

use pilotage_client_session::{StreamId, TransportEvent};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use wtransport::tls::Sha256Digest;
use wtransport::{ClientConfig, Connection, Endpoint, RecvStream, SendStream};

use crate::error::PilotError;

/// Deadline for each connection step, in seconds.
const CONNECT_TIMEOUT_S: u64 = 10;
/// Time the reader tasks get to stop before they are aborted.
const SHUTDOWN_GRACE: Duration = Duration::from_millis(500);
/// Events the pilot loop may fall behind by before readers wait.
const EVENT_QUEUE: usize = 1024;
const READ_BUFFER_BYTES: usize = 64 * 1024;

/// An open link to the host.
pub(crate) struct Link {
    connection: Arc<Connection>,
    bootstrap: SendStream,
    readers: Vec<(&'static str, JoinHandle<()>)>,
}

impl Link {
    /// Connects, opens the bootstrap stream and starts the reader tasks.
    pub(crate) async fn connect(
        url: &str,
        cert_sha256: [u8; 32],
    ) -> Result<(Self, mpsc::Receiver<TransportEvent>), PilotError> {
        let config = ClientConfig::builder()
            .with_bind_default()
            .with_server_certificate_hashes([Sha256Digest::new(cert_sha256)])
            .build();
        let endpoint = Endpoint::client(config).map_err(PilotError::Endpoint)?;
        let connection = within("connect", endpoint.connect(url))
            .await?
            .map_err(|source| PilotError::Connect {
                url: url.to_owned(),
                source,
            })?;
        let opening = within("open the bootstrap stream", connection.open_bi())
            .await?
            .map_err(|source| stream_error("bootstrap", source))?;
        let (bootstrap, bootstrap_recv) = within("finish the bootstrap stream", opening)
            .await?
            .map_err(|source| stream_error("bootstrap", source))?;

        let connection = Arc::new(connection);
        let (events, receiver) = mpsc::channel(EVENT_QUEUE);
        let readers = vec![
            (
                "bootstrap-reader",
                tokio::spawn(read_bootstrap(bootstrap_recv, events.clone())),
            ),
            (
                "uni-acceptor",
                tokio::spawn(accept_uni_streams(Arc::clone(&connection), events.clone())),
            ),
            (
                "datagram-reader",
                tokio::spawn(read_datagrams(Arc::clone(&connection), events)),
            ),
        ];
        let link = Self {
            connection,
            bootstrap,
            readers,
        };
        Ok((link, receiver))
    }

    /// Writes bytes to the reliable bootstrap stream.
    pub(crate) async fn send_bootstrap(&mut self, bytes: &[u8]) -> Result<(), PilotError> {
        self.bootstrap
            .write_all(bytes)
            .await
            .map_err(|source| PilotError::LinkDown {
                stage: "bootstrap write",
                source: Box::new(source),
            })
    }

    /// Sends one datagram.
    pub(crate) fn send_datagram(&self, bytes: &[u8]) -> Result<(), PilotError> {
        self.connection
            .send_datagram(bytes)
            .map_err(|source| PilotError::LinkDown {
                stage: "datagram send",
                source: Box::new(source),
            })
    }

    /// Closes the connection and stops the reader tasks. A task that does
    /// not stop in the grace time is named in the log and aborted.
    pub(crate) async fn shutdown(self) {
        self.connection.close(0_u32.into(), b"pilot done");
        for (name, mut reader) in self.readers {
            if tokio::time::timeout(SHUTDOWN_GRACE, &mut reader)
                .await
                .is_err()
            {
                tracing::warn!(task = name, "reader task did not stop; aborting it");
                reader.abort();
            }
        }
    }
}

async fn within<F: std::future::Future>(
    step: &'static str,
    future: F,
) -> Result<F::Output, PilotError> {
    tokio::time::timeout(Duration::from_secs(CONNECT_TIMEOUT_S), future)
        .await
        .map_err(|_| PilotError::HostTimeout {
            step,
            seconds: CONNECT_TIMEOUT_S,
        })
}

fn stream_error(
    stream: &'static str,
    source: impl std::error::Error + Send + Sync + 'static,
) -> PilotError {
    PilotError::Stream {
        stream,
        source: Box::new(source),
    }
}

/// Reports the loss once. A closed receiver means the pilot loop is gone,
/// and then nobody is left to tell.
async fn report_lost(events: &mpsc::Sender<TransportEvent>, detail: String) {
    events
        .send(TransportEvent::TransportLost { detail })
        .await
        .ok();
}

async fn read_bootstrap(mut recv: RecvStream, events: mpsc::Sender<TransportEvent>) {
    let mut buffer = vec![0_u8; READ_BUFFER_BYTES];
    loop {
        match recv.read(&mut buffer).await {
            Ok(Some(count)) => {
                let bytes = buffer.get(..count).map(<[u8]>::to_vec).unwrap_or_default();
                if events
                    .send(TransportEvent::BootstrapReceived(bytes))
                    .await
                    .is_err()
                {
                    return;
                }
            }
            Ok(None) => return report_lost(&events, "bootstrap stream ended".into()).await,
            Err(error) => return report_lost(&events, format!("bootstrap read: {error}")).await,
        }
    }
}

async fn accept_uni_streams(connection: Arc<Connection>, events: mpsc::Sender<TransportEvent>) {
    let mut next_id = 0_u64;
    loop {
        match connection.accept_uni().await {
            Ok(recv) => {
                next_id = next_id.wrapping_add(1);
                let id = StreamId(next_id);
                if events
                    .send(TransportEvent::UniStreamOpened(id))
                    .await
                    .is_err()
                {
                    return;
                }
                tokio::spawn(read_uni_stream(id, recv, events.clone()));
            }
            Err(error) => return report_lost(&events, format!("accept_uni: {error}")).await,
        }
    }
}

async fn read_uni_stream(id: StreamId, mut recv: RecvStream, events: mpsc::Sender<TransportEvent>) {
    let mut buffer = vec![0_u8; READ_BUFFER_BYTES];
    loop {
        let event = match recv.read(&mut buffer).await {
            Ok(Some(count)) => {
                let bytes = buffer.get(..count).map(<[u8]>::to_vec).unwrap_or_default();
                TransportEvent::UniStreamReceived(id, bytes)
            }
            // One stream ending is not the link ending.
            Ok(None) | Err(_) => TransportEvent::UniStreamClosed(id),
        };
        let closed = matches!(event, TransportEvent::UniStreamClosed(_));
        if events.send(event).await.is_err() || closed {
            return;
        }
    }
}

async fn read_datagrams(connection: Arc<Connection>, events: mpsc::Sender<TransportEvent>) {
    loop {
        match connection.receive_datagram().await {
            Ok(datagram) => {
                let event = TransportEvent::DatagramReceived(datagram.payload().to_vec());
                // Telemetry is periodic. A full queue drops this sample
                // and keeps the reader running; the next sample replaces
                // it. A closed queue means the pilot loop is gone.
                if let Err(mpsc::error::TrySendError::Closed(_)) = events.try_send(event) {
                    return;
                }
            }
            Err(error) => return report_lost(&events, format!("datagram read: {error}")).await,
        }
    }
}
