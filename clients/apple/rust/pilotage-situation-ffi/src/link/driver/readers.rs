//! The connection's reader tasks: each moves one transport lane's bytes
//! into the driver's event channel and reports the lane's end, nothing
//! more — interpretation stays with the engine.

use pilotage_client_session::{StreamId, TransportEvent};
use tokio::sync::mpsc;
use wtransport::Connection;

use super::ReaderEvent;

/// Reads the bootstrap stream until it ends.
pub(super) fn spawn_bootstrap_reader(
    mut recv: wtransport::RecvStream,
    events: mpsc::UnboundedSender<ReaderEvent>,
) {
    tokio::spawn(async move {
        let mut buf = vec![0_u8; 8192];
        loop {
            match recv.read(&mut buf).await {
                Ok(Some(read)) => {
                    let event = TransportEvent::BootstrapReceived(buf[..read].to_vec());
                    if events.send(ReaderEvent::Transport(event)).is_err() {
                        return;
                    }
                }
                Ok(None) | Err(_) => {
                    events
                        .send(ReaderEvent::Transport(TransportEvent::TransportLost {
                            detail: "bootstrap stream ended".to_owned(),
                        }))
                        .ok();
                    return;
                }
            }
        }
    });
}

/// Accepts host-initiated uni streams and spawns a reader per stream.
pub(super) fn spawn_uni_acceptor(
    connection: Connection,
    events: mpsc::UnboundedSender<ReaderEvent>,
) {
    tokio::spawn(async move {
        let mut next_stream = 0_u64;
        loop {
            let Ok(mut recv) = connection.accept_uni().await else {
                return;
            };
            next_stream = next_stream.wrapping_add(1);
            let stream = StreamId(next_stream);
            if events
                .send(ReaderEvent::Transport(TransportEvent::UniStreamOpened(
                    stream,
                )))
                .is_err()
            {
                return;
            }
            let events = events.clone();
            tokio::spawn(async move {
                let mut buf = vec![0_u8; 8192];
                loop {
                    match recv.read(&mut buf).await {
                        Ok(Some(read)) => {
                            let event =
                                TransportEvent::UniStreamReceived(stream, buf[..read].to_vec());
                            if events.send(ReaderEvent::Transport(event)).is_err() {
                                return;
                            }
                        }
                        Ok(None) | Err(_) => {
                            events
                                .send(ReaderEvent::Transport(TransportEvent::UniStreamClosed(
                                    stream,
                                )))
                                .ok();
                            return;
                        }
                    }
                }
            });
        }
    });
}

/// Reads datagrams until the connection ends.
pub(super) fn spawn_datagram_reader(
    connection: Connection,
    events: mpsc::UnboundedSender<ReaderEvent>,
) {
    tokio::spawn(async move {
        loop {
            match connection.receive_datagram().await {
                Ok(datagram) => {
                    let event = TransportEvent::DatagramReceived(datagram.payload().to_vec());
                    if events.send(ReaderEvent::Transport(event)).is_err() {
                        return;
                    }
                }
                Err(_) => {
                    events
                        .send(ReaderEvent::Transport(TransportEvent::TransportLost {
                            detail: "connection closed".to_owned(),
                        }))
                        .ok();
                    return;
                }
            }
        }
    });
}
