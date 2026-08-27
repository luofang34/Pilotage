use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Write};
use std::net::{SocketAddr, TcpListener, TcpStream, ToSocketAddrs};
use std::time::{Duration, Instant};

use crate::error::XPlaneTrialError;
use crate::identity::{
    ExpectedXPlaneIdentity, VerifiedXPlaneBinding, VerifiedXPlaneIdentity, binding, verify_blocking,
};
use crate::protocol::{Message, parse_line};
use crate::{Digest, XPlaneTruthSample};
use pilotage_trial::AppliedWind;

mod sample_stream;
mod session_state;

pub use session_state::SessionReceipt;
use session_state::State;

/// A local listener for the X-Plane trial plugin.
#[derive(Debug)]
pub struct XPlaneTrialListener {
    listener: TcpListener,
}

impl XPlaneTrialListener {
    /// Binds a local trial listener.
    ///
    /// # Errors
    ///
    /// Returns an error when the address is invalid or unavailable.
    pub fn bind_blocking(address: impl ToSocketAddrs) -> Result<Self, XPlaneTrialError> {
        let addresses = address
            .to_socket_addrs()
            .map_err(|source| XPlaneTrialError::Bind {
                address: "unresolved address".to_owned(),
                source,
            })?
            .collect::<Vec<_>>();
        if let Some(address) = addresses.iter().find(|address| !address.ip().is_loopback()) {
            return Err(XPlaneTrialError::NonLocalAddress {
                address: address.to_string(),
            });
        }
        let label = addresses
            .first()
            .map_or_else(|| "empty address".to_owned(), ToString::to_string);
        let listener =
            TcpListener::bind(addresses.as_slice()).map_err(|source| XPlaneTrialError::Bind {
                address: label,
                source,
            })?;
        listener
            .set_nonblocking(true)
            .map_err(|source| XPlaneTrialError::Listener {
                operation: "set nonblocking mode",
                source,
            })?;
        Ok(Self { listener })
    }

    /// Returns the bound local address.
    ///
    /// # Errors
    ///
    /// Returns an error when the socket cannot report its address.
    pub fn local_addr(&self) -> Result<SocketAddr, XPlaneTrialError> {
        self.listener
            .local_addr()
            .map_err(|source| XPlaneTrialError::Listener {
                operation: "read local address",
                source,
            })
    }

    /// Waits for a plugin and verifies all active plant files.
    ///
    /// # Errors
    ///
    /// Returns an error for timeout, transport, protocol, or identity failure.
    pub fn accept_verified_blocking(
        &self,
        expected: &ExpectedXPlaneIdentity,
        timeout: Duration,
    ) -> Result<XPlaneTrialSession, XPlaneTrialError> {
        let deadline = Instant::now() + timeout;
        let stream = loop {
            match self.listener.accept() {
                Ok((stream, _)) => break stream,
                Err(source) if source.kind() == std::io::ErrorKind::WouldBlock => {
                    if Instant::now() >= deadline {
                        return Err(XPlaneTrialError::Listener {
                            operation: "accept before timeout",
                            source: std::io::Error::new(
                                std::io::ErrorKind::TimedOut,
                                "the X-Plane trial plugin did not connect",
                            ),
                        });
                    }
                    std::thread::park_timeout(Duration::from_millis(1));
                }
                Err(source) => {
                    return Err(XPlaneTrialError::Listener {
                        operation: "accept connection",
                        source,
                    });
                }
            }
        };
        XPlaneTrialSession::verify_blocking(stream, expected, timeout)
    }
}

/// One verified and stateful connection to the X-Plane trial plugin.
#[derive(Debug)]
pub struct XPlaneTrialSession {
    writer: TcpStream,
    reader: BufReader<TcpStream>,
    binding: VerifiedXPlaneBinding,
    state: State,
    last_sequence: Option<u64>,
    last_sim_time_s: Option<f64>,
    pending_samples: VecDeque<XPlaneTruthSample>,
}

impl XPlaneTrialSession {
    fn verify_blocking(
        stream: TcpStream,
        expected: &ExpectedXPlaneIdentity,
        timeout: Duration,
    ) -> Result<Self, XPlaneTrialError> {
        stream
            .set_nonblocking(false)
            .map_err(|source| session_io("set blocking mode", source))?;
        stream
            .set_read_timeout(Some(timeout))
            .map_err(|source| session_io("set read timeout", source))?;
        stream
            .set_write_timeout(Some(timeout))
            .map_err(|source| session_io("set write timeout", source))?;
        let reader_stream = stream
            .try_clone()
            .map_err(|source| session_io("clone stream", source))?;
        let mut session = Self {
            writer: stream,
            reader: BufReader::new(reader_stream),
            binding: binding(placeholder_identity(expected.simulator_model_digest)),
            state: State::Connected,
            last_sequence: None,
            last_sim_time_s: None,
            pending_samples: VecDeque::new(),
        };
        let hello = match session.read_message_blocking("read HELLO")? {
            Message::Hello(hello) => hello,
            _ => return receipt_mismatch("the first message is not HELLO"),
        };
        session.binding = binding(verify_blocking(expected, &hello)?);
        Ok(session)
    }

    /// Returns the verified runtime identity.
    #[must_use]
    pub const fn identity(&self) -> &VerifiedXPlaneIdentity {
        self.binding.identity()
    }

    /// Returns the verified simulator capability for adapter binding.
    #[must_use]
    pub const fn binding(&self) -> &VerifiedXPlaneBinding {
        &self.binding
    }

    /// Stores the exact scenario and condition identities in the plugin.
    ///
    /// # Errors
    ///
    /// Returns an error when the session state or receipt is invalid.
    pub fn configure_blocking(
        &mut self,
        generation: u64,
        scenario: Digest,
        condition: Digest,
    ) -> Result<SessionReceipt, XPlaneTrialError> {
        if generation == 0 || !matches!(self.state, State::Connected | State::Stopped) {
            return invalid_state("configure");
        }
        self.send_blocking(&format!("CONFIG {generation} {scenario} {condition}"))?;
        match self.read_command_message_blocking("configure")? {
            Message::Configured {
                generation: actual,
                scenario: actual_scenario,
                condition: actual_condition,
            } if actual == generation
                && actual_scenario == scenario
                && actual_condition == condition =>
            {
                self.state = State::Configured {
                    generation,
                    scenario,
                    condition,
                };
                Ok(SessionReceipt::Configured { generation })
            }
            _ => receipt_mismatch("CONFIGURED does not match CONFIG"),
        }
    }

    /// Starts one configured trial generation.
    ///
    /// # Errors
    ///
    /// Returns an error when the generation or receipt does not match.
    pub fn start_blocking(&mut self) -> Result<SessionReceipt, XPlaneTrialError> {
        let State::Configured { generation, .. } = self.state else {
            return invalid_state("start");
        };
        self.send_blocking(&format!("START {generation}"))?;
        match self.read_command_message_blocking("start")? {
            Message::Started {
                generation: actual,
                sim_time_s,
                reset_generation,
            } if actual == generation => {
                self.state = State::Active {
                    generation,
                    reset_generation,
                };
                self.last_sequence = None;
                self.last_sim_time_s = None;
                self.pending_samples.clear();
                Ok(SessionReceipt::Started {
                    generation,
                    sim_time_s,
                    reset_generation,
                })
            }
            _ => receipt_mismatch("STARTED does not match START"),
        }
    }

    /// Stops the active trial and checks its complete sample count.
    ///
    /// # Errors
    ///
    /// Returns an error when samples were lost or the receipt does not match.
    pub fn stop_blocking(&mut self) -> Result<SessionReceipt, XPlaneTrialError> {
        let State::Active { generation, .. } = self.state else {
            return invalid_state("stop");
        };
        self.send_blocking(&format!("STOP {generation}"))?;
        match self.read_command_message_blocking("stop")? {
            Message::Stopped {
                generation: actual,
                sample_count,
                sim_time_s,
            } if actual == generation && sample_count == self.completed_sample_count() => {
                self.state = State::Stopped;
                Ok(SessionReceipt::Stopped {
                    generation,
                    sample_count,
                    sim_time_s,
                })
            }
            _ => receipt_mismatch("STOPPED does not match the received sample count"),
        }
    }

    /// Requests and waits for a simulator reset while no trial is active.
    ///
    /// # Errors
    ///
    /// Returns an error when the state or receipt is invalid.
    pub fn reset_blocking(&mut self, generation: u64) -> Result<SessionReceipt, XPlaneTrialError> {
        if generation == 0 || matches!(self.state, State::Active { .. }) {
            return invalid_state("reset");
        }
        self.send_blocking(&format!("RESET {generation}"))?;
        if !matches!(
            self.read_command_message_blocking("accept reset")?,
            Message::Resetting { generation: actual } if actual == generation
        ) {
            return receipt_mismatch("RESETTING does not match RESET");
        }
        match self.read_command_message_blocking("complete reset")? {
            Message::ResetComplete {
                generation: actual,
                reset_generation,
                sim_time_s,
            } if actual == generation && reset_generation != 0 => {
                self.state = State::Stopped;
                self.last_sequence = None;
                self.last_sim_time_s = None;
                self.pending_samples.clear();
                Ok(SessionReceipt::ResetComplete {
                    generation,
                    reset_generation,
                    sim_time_s,
                })
            }
            _ => receipt_mismatch("RESET_COMPLETE does not match RESET"),
        }
    }

    /// Applies one resolved deterministic wind request.
    ///
    /// The caller calculates the wind from the condition artifact and
    /// simulator time. The plugin reports the actual local wind separately.
    ///
    /// # Errors
    ///
    /// Returns an error when the state, request, or receipt is invalid.
    pub fn set_wind_blocking(
        &mut self,
        condition_generation: u32,
        requested: AppliedWind,
    ) -> Result<SessionReceipt, XPlaneTrialError> {
        let generation = match self.state {
            State::Configured { generation, .. } | State::Active { generation, .. } => generation,
            State::Connected | State::Stopped => return invalid_state("set wind"),
        };
        if condition_generation == 0
            || condition_generation > 16_777_215
            || !requested.speed_mps.is_finite()
            || !(0.0..=50.0).contains(&requested.speed_mps)
            || !requested.direction_deg.is_finite()
            || !(0.0..=360.0).contains(&requested.direction_deg)
        {
            return receipt_mismatch("wind request is outside the X-Plane boundary");
        }
        self.send_blocking(&format!(
            "WIND {generation} {condition_generation} {} {}",
            requested.speed_mps, requested.direction_deg
        ))?;
        match self.read_command_message_blocking("set wind")? {
            Message::WindApplied {
                generation: actual,
                condition_generation: actual_condition,
                actual_speed_mps,
                actual_direction_deg,
            } if actual == generation && actual_condition == condition_generation => {
                Ok(SessionReceipt::WindApplied {
                    generation,
                    condition_generation,
                    requested,
                    actual_speed_mps,
                    actual_direction_deg,
                })
            }
            _ => receipt_mismatch("WIND_APPLIED does not match WIND"),
        }
    }

    fn send_blocking(&mut self, command: &str) -> Result<(), XPlaneTrialError> {
        self.writer
            .write_all(command.as_bytes())
            .and_then(|()| self.writer.write_all(b"\n"))
            .and_then(|()| self.writer.flush())
            .map_err(|source| session_io("write command", source))
    }

    fn read_command_message_blocking(
        &mut self,
        operation: &'static str,
    ) -> Result<Message, XPlaneTrialError> {
        loop {
            match self.read_message_blocking(operation)? {
                Message::Sample(sample) => self.buffer_sample(sample)?,
                Message::Error { generation, code } => {
                    return Err(XPlaneTrialError::CommandRejected { generation, code });
                }
                Message::AircraftChanged | Message::Hello(_) | Message::Active { .. } => {
                    return receipt_mismatch("the running simulator identity changed");
                }
                Message::Rewind { .. } => {
                    return receipt_mismatch("the simulator time rewound");
                }
                message => return Ok(message),
            }
        }
    }

    fn read_message_blocking(
        &mut self,
        operation: &'static str,
    ) -> Result<Message, XPlaneTrialError> {
        let mut line = String::new();
        let size = self
            .reader
            .read_line(&mut line)
            .map_err(|source| session_io(operation, source))?;
        if size == 0 {
            return Err(XPlaneTrialError::PeerClosed { operation });
        }
        parse_line(line.trim_end_matches(['\r', '\n']))
    }
}

fn placeholder_identity(model: Digest) -> VerifiedXPlaneIdentity {
    VerifiedXPlaneIdentity {
        protocol_version: 0,
        xplane_version: 0,
        sdk_version: 0,
        host_application_id: 0,
        trial_source_build_id: String::new(),
        aircraft_digest: Digest::from_bytes([0; 32]),
        trial_plugin_digest: Digest::from_bytes([0; 32]),
        bridge_plugin_digest: Digest::from_bytes([0; 32]),
        bridge_config_digest: Digest::from_bytes([0; 32]),
        simulator_model_digest: model,
        binding_digest: Digest::from_bytes([0; 32]),
    }
}

fn session_io(operation: &'static str, source: std::io::Error) -> XPlaneTrialError {
    XPlaneTrialError::SessionIo { operation, source }
}

fn invalid_state<T>(operation: &'static str) -> Result<T, XPlaneTrialError> {
    Err(XPlaneTrialError::InvalidState { operation })
}

fn receipt_mismatch<T>(detail: impl Into<String>) -> Result<T, XPlaneTrialError> {
    Err(XPlaneTrialError::ReceiptMismatch {
        detail: detail.into(),
    })
}
