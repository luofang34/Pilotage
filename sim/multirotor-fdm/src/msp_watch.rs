//! Whether a running Betaflight still answers MSP.
//!
//! A Betaflight SITL can hang with its process alive and its TCP serial
//! ports still accepting connections while no MSP command is answered.
//! The host's adapter then reconnects forever and the vehicle is gone.
//! The sidecar watches for it on a serial port of its own (UART2, TCP
//! 5762, configured for MSP in `sim/betaflight/sitl.cli`), so it never
//! competes with the adapter's link on UART1: it asks for the API version
//! every [`REQUEST_PERIOD`] and reports a hang when no reply has come for
//! [`SILENCE_LIMIT`].

use std::io::{ErrorKind, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::{Duration, Instant};

#[cfg(test)]
mod tests;

/// Betaflight's TCP port for UART2.
pub const WATCH_PORT: u16 = 5_762;
/// How often the watch asks.
pub const REQUEST_PERIOD: Duration = Duration::from_millis(500);
/// How long without a reply counts as a hang.
pub const SILENCE_LIMIT: Duration = Duration::from_secs(5);
/// How long one connection attempt may take.
const CONNECT_TIMEOUT: Duration = Duration::from_millis(200);

/// An MSP v1 `MSP_API_VERSION` request: header, no payload, command 1,
/// checksum 1.
const API_VERSION_REQUEST: [u8; 6] = *b"$M<\x00\x01\x01";
/// The start of every MSP v1 reply.
const REPLY_HEADER: &[u8] = b"$M>";

/// The watch on one Betaflight's MSP answers.
#[derive(Debug)]
pub struct MspWatch {
    addr: SocketAddr,
    stream: Option<TcpStream>,
    // When this connection last heard a reply, or opened.
    heard: Option<Instant>,
    last_request: Option<Instant>,
    // Bytes of a reply header matched so far, across reads.
    matched: usize,
}

impl MspWatch {
    /// A watch on the MSP port at `addr`.
    #[must_use]
    pub fn new(addr: SocketAddr) -> Self {
        Self {
            addr,
            stream: None,
            heard: None,
            last_request: None,
            matched: 0,
        }
    }

    /// Forgets the connection: a relaunched Betaflight is watched afresh.
    pub fn reset(&mut self) {
        *self = Self::new(self.addr);
    }

    /// One step at `now`: connects if needed, reads any reply, and asks
    /// again when due. Returns `true` once the connection is up and no
    /// reply has come for [`SILENCE_LIMIT`].
    ///
    /// A port that refuses connections is not a hang: Betaflight that
    /// stopped serving the port exits or never bound it, and the
    /// supervisor's other checks see that.
    pub fn poll(&mut self, now: Instant) -> bool {
        if self.stream.is_none() {
            let Ok(stream) = TcpStream::connect_timeout(&self.addr, CONNECT_TIMEOUT) else {
                return false;
            };
            if stream.set_nonblocking(true).is_err() {
                return false;
            }
            self.stream = Some(stream);
            self.heard = Some(now);
            self.last_request = None;
            self.matched = 0;
        }
        if self.read_replies() {
            self.heard = Some(now);
        }
        if self
            .last_request
            .is_none_or(|at| now.saturating_duration_since(at) >= REQUEST_PERIOD)
        {
            self.last_request = Some(now);
            let sent = self
                .stream
                .as_mut()
                .is_some_and(|stream| stream.write_all(&API_VERSION_REQUEST).is_ok());
            if !sent {
                // The peer closed: connect again next step.
                self.stream = None;
                return false;
            }
        }
        self.heard
            .is_some_and(|heard| now.saturating_duration_since(heard) >= SILENCE_LIMIT)
    }

    /// Drains the socket; whether a reply header arrived.
    fn read_replies(&mut self) -> bool {
        let Some(stream) = self.stream.as_mut() else {
            return false;
        };
        let mut buffer = [0_u8; 256];
        let mut replied = false;
        loop {
            match stream.read(&mut buffer) {
                Ok(0) => {
                    self.stream = None;
                    return replied;
                }
                Ok(len) => {
                    for byte in &buffer[..len] {
                        if REPLY_HEADER.get(self.matched) == Some(byte) {
                            self.matched = self.matched.wrapping_add(1);
                            if self.matched == REPLY_HEADER.len() {
                                replied = true;
                                self.matched = 0;
                            }
                        } else {
                            self.matched = usize::from(*byte == REPLY_HEADER[0]);
                        }
                    }
                }
                Err(error) if error.kind() == ErrorKind::WouldBlock => return replied,
                Err(_) => {
                    self.stream = None;
                    return replied;
                }
            }
        }
    }
}
