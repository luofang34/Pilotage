//! Bounded event transport for live simulation procedures.

use std::time::{Duration, Instant};

use pilotage_client_session::{ClientAction, ControlCommand, StreamId, TransportEvent};
use pilotage_protocol::wire;

use super::Client;

const CONTROL_INTERVAL: Duration = Duration::from_millis(50);

impl Client {
    pub(crate) fn sampled_at(&self) -> u64 {
        u64::try_from(self.started.elapsed().as_nanos()).unwrap_or(u64::MAX)
    }

    pub(crate) async fn execute_checked(
        &mut self,
        actions: Vec<ClientAction>,
    ) -> Result<(), String> {
        for action in actions {
            match action {
                ClientAction::SendBootstrap(bytes) => {
                    tokio::time::timeout(super::STEP_TIMEOUT, self.send.write_all(&bytes))
                        .await
                        .map_err(|_| "bootstrap write timed out".to_owned())?
                        .map_err(|error| format!("bootstrap write: {error}"))?;
                }
                ClientAction::SendDatagram(bytes) => self
                    .connection
                    .send_datagram(bytes)
                    .map_err(|error| format!("control datagram: {error}"))?,
                ClientAction::Emit(event) => self.observe(event),
                ClientAction::Stop(fault) => return Err(format!("client stopped: {fault:?}")),
                ClientAction::ScheduleReconnect { .. } => {
                    return Err("live simulation connection was lost".to_owned());
                }
            }
        }
        Ok(())
    }

    pub(crate) async fn neutral(&mut self) -> Result<(), String> {
        let Some((vehicle, scope)) = self.engine.control_target() else {
            return Ok(());
        };
        let intent = wire::ControlIntent {
            family: Some(wire::control_intent::Family::Velocity(
                wire::VelocityIntent {
                    frame: wire::ReferenceFrame::BodyFrd as i32,
                    vx: 0.0,
                    vy: 0.0,
                    vz: 0.0,
                    yaw_rate: 0.0,
                },
            )),
        };
        let actions = self.engine.control_frame(
            vehicle,
            &scope,
            ControlCommand::Intent(intent),
            self.sampled_at(),
        );
        self.execute_checked(actions).await
    }

    /// Services telemetry and both reliable streams while maintaining neutral input.
    pub(crate) async fn progress(&mut self, deadline: Instant) -> Result<(), String> {
        if Instant::now() >= deadline {
            return Err("live procedure deadline expired".to_owned());
        }
        if Instant::now() >= self.next_neutral_at {
            self.neutral().await?;
            self.next_neutral_at = Instant::now() + CONTROL_INTERVAL;
        }
        let mut bootstrap = [0_u8; 16_384];
        let mut authority = [0_u8; 16_384];
        let next = deadline.min(self.next_neutral_at);
        let event = tokio::select! {
            result = self.recv.read(&mut bootstrap) => {
                let size = result.map_err(|error| format!("bootstrap read: {error}"))?
                    .ok_or("bootstrap stream closed")?;
                Some(TransportEvent::BootstrapReceived(bootstrap[..size].to_vec()))
            }
            result = self.connection.receive_datagram() => {
                let data = result.map_err(|error| format!("telemetry datagram: {error}"))?;
                Some(TransportEvent::DatagramReceived(data.payload().to_vec()))
            }
            result = self.connection.accept_uni(), if self.authority_recv.is_none() => {
                self.authority_recv = Some(result.map_err(|error| format!("authority stream: {error}"))?);
                Some(TransportEvent::UniStreamOpened(StreamId(1)))
            }
            result = authority_read(&mut self.authority_recv, &mut authority) => {
                let size = result?;
                Some(TransportEvent::UniStreamReceived(StreamId(1), authority[..size].to_vec()))
            }
            () = tokio::time::sleep_until(next.into()) => None,
        };
        if let Some(event) = event {
            let actions = self.engine.handle(event, self.sampled_at() / 1_000_000);
            self.execute_checked(actions).await?;
        }
        Ok(())
    }
}

async fn authority_read(
    stream: &mut Option<wtransport::RecvStream>,
    bytes: &mut [u8],
) -> Result<usize, String> {
    let Some(stream) = stream else {
        return std::future::pending().await;
    };
    stream
        .read(bytes)
        .await
        .map_err(|error| format!("authority read: {error}"))?
        .ok_or_else(|| "authority stream closed".to_owned())
}
