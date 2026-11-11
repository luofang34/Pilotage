//! Where uplink frames go and where FC replies come from: a UDP socket
//! toward the FC, or in-memory queues whose bytes an embedding host moves.

use std::collections::VecDeque;
use std::net::SocketAddr;
#[cfg(feature = "net")]
use std::net::UdpSocket;

/// The largest FC reply datagram read from the socket.
#[cfg(feature = "net")]
const REPLY_BUFFER_BYTES: usize = 512;

/// The uplink's byte transport.
#[derive(Debug)]
pub(super) enum UplinkTransport {
    /// A nonblocking UDP socket.
    #[cfg(feature = "net")]
    Udp(UdpSocket),
    /// Frames sent and replies received, for the host to move.
    Memory {
        sent: Vec<Vec<u8>>,
        replies: VecDeque<Vec<u8>>,
    },
}

impl UplinkTransport {
    pub(super) fn memory() -> Self {
        Self::Memory {
            sent: Vec::new(),
            replies: VecDeque::new(),
        }
    }

    /// Sends one frame; false when the socket refused it. An in-memory
    /// transport has no target: the host addresses what it sends.
    #[cfg_attr(not(feature = "net"), allow(unused_variables))]
    pub(super) fn send(&mut self, frame: &[u8], target: SocketAddr) -> bool {
        match self {
            #[cfg(feature = "net")]
            Self::Udp(socket) => socket.send_to(frame, target).is_ok(),
            Self::Memory { sent, .. } => {
                sent.push(frame.to_vec());
                true
            }
        }
    }

    /// The next FC reply datagram, if one waits. Never blocks.
    pub(super) fn receive(&mut self) -> Option<Vec<u8>> {
        match self {
            #[cfg(feature = "net")]
            Self::Udp(socket) => {
                let mut buffer = [0u8; REPLY_BUFFER_BYTES];
                let (len, _) = socket.recv_from(&mut buffer).ok()?;
                Some(buffer.get(..len).unwrap_or(&[]).to_vec())
            }
            Self::Memory { replies, .. } => replies.pop_front(),
        }
    }

    /// Queues one FC reply datagram; false for a socket transport, which
    /// receives its own replies.
    pub(super) fn push_reply(&mut self, datagram: &[u8]) -> bool {
        match self {
            #[cfg(feature = "net")]
            Self::Udp(_) => false,
            Self::Memory { replies, .. } => {
                replies.push_back(datagram.to_vec());
                true
            }
        }
    }

    /// Takes the frames sent since the last call, in send order; empty
    /// for a socket transport.
    pub(super) fn take_sent(&mut self) -> Vec<Vec<u8>> {
        match self {
            #[cfg(feature = "net")]
            Self::Udp(_) => Vec::new(),
            Self::Memory { sent, .. } => core::mem::take(sent),
        }
    }

    /// The local socket address, for a socket transport.
    pub(super) fn local_addr(&self) -> std::io::Result<SocketAddr> {
        match self {
            #[cfg(feature = "net")]
            Self::Udp(socket) => socket.local_addr(),
            Self::Memory { .. } => Err(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "an in-memory uplink has no socket",
            )),
        }
    }
}
