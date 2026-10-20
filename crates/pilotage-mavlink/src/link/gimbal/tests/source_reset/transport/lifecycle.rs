use super::*;
use crate::GimbalRateDemand;

mod projection;

struct Fixture {
    sender: UdpSocket,
    receiver: UdpSocket,
    state: Arc<Mutex<LinkState>>,
    control: ManagerControl,
    commands: mpsc::Receiver<OutboundCommand>,
    rates: watch::Receiver<Option<GimbalRateDemand>>,
    sequence: u8,
}

impl Fixture {
    async fn new(restart: bool, owner: (u8, u8)) -> Self {
        let sender = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let receiver = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let state = Arc::new(Mutex::new(LinkState {
            reset_policy: ResetPolicy::SimulatorHeuristic,
            ..LinkState::default()
        }));
        let (commands, pending) = mpsc::channel(16);
        let (rates, observer) = watch::channel(None);
        let control = ManagerControl::new(commands, rates, 1, Some(KEY))
            .unwrap()
            .with_manager_state(state.clone());
        prepare_source_reset(&state, restart);
        discovered(&state, 100, owner);
        Self {
            sender,
            receiver,
            state,
            control,
            commands: pending,
            rates: observer,
            sequence: 0,
        }
    }

    async fn send_rate(&mut self, demand: GimbalRateDemand) {
        send_gimbal_rate(
            &self.sender,
            &mut self.sequence,
            Some(self.receiver.local_addr().unwrap()),
            Some(demand),
        )
        .await;
    }

    async fn send_command(&mut self, command: OutboundCommand) {
        send_outbound_command(
            &self.sender,
            &mut self.sequence,
            Some(self.receiver.local_addr().unwrap()),
            Some(command),
        )
        .await;
    }

    fn configure(&mut self, owner: f32) -> OutboundCommand {
        while let Ok(command) = self.commands.try_recv() {
            if command.command == 1001 {
                assert_eq!(command.params[0], owner);
                return command;
            }
            assert_eq!(command.command, 512);
        }
        panic!("manager configure command was not queued");
    }

    async fn assert_no_control_write(&mut self) {
        let demand = self.rates.borrow().clone();
        if let Some(demand) = demand {
            self.send_rate(demand).await;
        }
        while let Ok(command) = self.commands.try_recv() {
            // Read-only probes are independent of the retired command responsibility.
            if command.command != 512 {
                self.send_command(command).await;
            }
        }
        sentinel(&self.sender, &self.receiver, &mut self.sequence).await;
    }

    fn reset_and_rediscover(&mut self, restart: bool, owner: (u8, u8)) {
        finish_source_reset(&self.state, restart);
        discovered(&self.state, 103, owner);
        self.control.maintain();
    }

    async fn sent_rate(&mut self) -> crate::PayloadSnapshot {
        assert!(self.control.rate_demand(0.5, 0.0));
        let rate = self.rates.borrow().clone().unwrap();
        self.send_rate(rate).await;
        assert_eq!(receive_message(&self.receiver).await, 282);
        let sent = self.control.snapshot_cached().unwrap();
        assert!(sent.progress.sent_at.is_some());
        sent
    }
}

async fn sent_rate_reset(restart: bool, queued_stop: bool) {
    let mut fixture = Fixture::new(restart, OWNER).await;
    let sent = fixture.sent_rate().await;
    let stop = queued_stop.then(|| {
        assert!(fixture.control.queue_link_loss_stop());
        fixture.rates.borrow().clone().unwrap()
    });
    fixture.reset_and_rediscover(restart, OWNER);
    fixture.assert_no_control_write().await;
    let retired = fixture.control.snapshot_cached().unwrap();
    assert_eq!(retired.command.identity, sent.command.identity);
    assert_eq!(retired.progress.sent_at, sent.progress.sent_at);
    assert_eq!(
        retired.progress.ended,
        Some(turret_core::EndReason::Revoked)
    );
    assert!(retired.progress.observed_complete_at.is_none());
    if let Some(stop) = stop {
        assert!(stop.send_before.unwrap() > Instant::now());
        let deadline = stop.send_before.unwrap();
        fixture.send_rate(stop).await;
        fixture.assert_no_control_write().await;
        // Exercise the retry boundary at its actual deadline, without polling.
        tokio::time::sleep_until(deadline.into()).await;
        fixture.control.maintain();
        fixture.assert_no_control_write().await;
    }
    fixture.control.bind_timing(None);
    assert!(fixture.control.rate_demand(0.5, 0.0));
    let rate = fixture.rates.borrow().clone().unwrap();
    fixture.send_rate(rate).await;
    assert_eq!(receive_message(&fixture.receiver).await, 282);
    let fresh = fixture.control.snapshot_cached().unwrap();
    assert_ne!(fresh.command.identity.epoch, sent.command.identity.epoch);
    assert!(fresh.progress.sent_at.is_some());
}

async fn claim_reset(restart: bool, sent: bool) {
    let mut fixture = Fixture::new(restart, (0, 0)).await;
    assert!(!fixture.control.rate_demand(0.5, 0.0));
    let claim = fixture.configure(f32::from(OWNER.0));
    let receipt = claim.receipt.clone().unwrap();
    let deadline = claim.send_before.unwrap();
    let queued = if sent {
        fixture.send_command(claim).await;
        assert_eq!(receive_message(&fixture.receiver).await, 76);
        assert!(receipt.sent_at_cached().is_some());
        None
    } else {
        Some(claim)
    };
    fixture.reset_and_rediscover(restart, (0, 0));
    if let Some(claim) = queued {
        assert!(deadline > Instant::now());
        fixture.send_command(claim).await;
        assert!(receipt.sent_at_cached().is_none());
    }
    fixture.assert_no_control_write().await;
    tokio::time::sleep_until(deadline.into()).await;
    fixture.control.maintain();
    fixture.assert_no_control_write().await;
    assert!(fixture.control.snapshot_cached().is_none());
}

async fn release_reset(restart: bool) {
    let mut fixture = Fixture::new(restart, OWNER).await;
    fixture.sent_rate().await;
    assert!(fixture.control.queue_link_loss_stop());
    let stop = fixture.rates.borrow().clone().unwrap();
    let stop_receipt = stop.receipt.clone().unwrap();
    fixture.send_rate(stop).await;
    assert_eq!(receive_message(&fixture.receiver).await, 282);
    assert!(stop_receipt.sent_at_cached().is_some());
    fixture.control.maintain();
    let release = fixture.configure(-3.0);
    let receipt = release.receipt.clone().unwrap();
    let deadline = release.send_before.unwrap();
    fixture.reset_and_rediscover(restart, OWNER);
    assert!(deadline > Instant::now());
    fixture.send_command(release).await;
    assert!(receipt.sent_at_cached().is_none());
    fixture.assert_no_control_write().await;
    tokio::time::sleep_until(deadline.into()).await;
    fixture.control.maintain();
    fixture.assert_no_control_write().await;
}

#[test]
fn sent_rate_and_stop_retries_remain_with_their_original_attachment() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        for restart in [false, true] {
            for queued_stop in [false, true] {
                sent_rate_reset(restart, queued_stop).await;
            }
        }
    });
}

#[test]
fn queued_and_sent_claims_and_queued_releases_cannot_cross_the_source_epoch() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        for restart in [false, true] {
            for sent in [false, true] {
                claim_reset(restart, sent).await;
            }
            release_reset(restart).await;
        }
    });
}
