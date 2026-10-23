//! Authenticate the first bootstrap frame before creating an engine client.

use std::time::Duration;
use std::{future::Future, task::Poll};

use pilotage_protocol::{ADMISSION_REJECTED_CLOSE_CODE, ClientHello};
use pilotage_session::DomainEnvelope;
use wtransport::{Connection, RecvStream, SendStream};

use crate::runtime::wire_codec::{
    InboundBootstrap, MAX_BOOTSTRAP_FRAME_LEN, complete_frame_len, decode_bootstrap_message,
};

/// One budget covers waiting for the stream and assembling the first frame.
const ADMISSION_TIMEOUT: Duration = Duration::from_secs(3);

pub(super) struct AdmittedBootstrap {
    pub(super) send: SendStream,
    pub(super) recv: RecvStream,
    pub(super) hello: ClientHello,
    pub(super) pending: Vec<u8>,
    /// The client presented the mission approver secret, or the host has
    /// none.
    pub(super) may_approve: bool,
}

pub(super) async fn admit(
    connection: &Connection,
    expected: &crate::runtime::Admission,
) -> Option<AdmittedBootstrap> {
    match tokio::time::timeout(ADMISSION_TIMEOUT, read_hello(connection, expected)).await {
        Ok(Some(bootstrap)) => Some(bootstrap),
        _ => {
            connection.close(
                wtransport::VarInt::from_u32(ADMISSION_REJECTED_CLOSE_CODE),
                b"admission rejected",
            );
            tracing::warn!("session admission rejected");
            None
        }
    }
}

async fn read_hello(
    connection: &Connection,
    expected: &crate::runtime::Admission,
) -> Option<AdmittedBootstrap> {
    let (send, mut recv) = connection.accept_bi().await.ok()?;
    let mut pending = Vec::new();
    let mut buffer = [0; 4096];
    loop {
        if let Some(size) = complete_frame_len(&pending).ok()? {
            let (message, _) = decode_bootstrap_message(pending.get(..size)?).ok()?;
            let InboundBootstrap::Engine(message) = message else {
                return None;
            };
            let DomainEnvelope::Hello(mut hello) = *message else {
                return None;
            };
            let may_approve = expected.check(&hello.join_token)?;
            discard_buffered_datagrams(connection).await?;
            // The session engine trusts the admitted transport, not the secret.
            hello.join_token.clear();
            pending.drain(..size);
            return Some(AdmittedBootstrap {
                send,
                recv,
                hello,
                pending,
                may_approve,
            });
        }
        if pending.len() > MAX_BOOTSTRAP_FRAME_LEN + 10 {
            return None;
        }
        let size = tokio::select! {
            biased;
            datagram = connection.receive_datagram() => {
                datagram.ok()?;
                continue;
            }
            read = recv.read(&mut buffer) => read.ok()??,
        };
        pending.extend_from_slice(buffer.get(..size)?);
    }
}

async fn discard_buffered_datagrams(connection: &Connection) -> Option<()> {
    loop {
        let datagram = connection.receive_datagram();
        tokio::pin!(datagram);
        let ready = std::future::poll_fn(|context| {
            Poll::Ready(match datagram.as_mut().poll(context) {
                Poll::Ready(result) => Some(result),
                Poll::Pending => None,
            })
        })
        .await;
        match ready {
            Some(Ok(_)) => {}
            Some(Err(_)) => return None,
            None => return Some(()),
        }
    }
}
