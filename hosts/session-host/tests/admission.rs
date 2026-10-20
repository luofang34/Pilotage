//! Session admission through the production WebTransport host.

#![cfg(test)]

#[path = "admission/client.rs"]
mod client;
use client::*;
use pilotage_protocol::{ADMISSION_REJECTED_CLOSE_CODE, JoinToken, wire};
use pilotage_session_host::cli::AdapterKind;
use pilotage_session_host::runtime::{self, NetworkOptions, RuntimeOptions};
use std::net::Ipv4Addr;
use wtransport::error::ConnectionError;

#[test]
fn default_bind_is_local_and_remote_bind_requires_credentials_before_startup() {
    runtime().block_on(async {
        let local = start_host(None).await;
        assert_eq!(local.local_addr.ip(), Ipv4Addr::LOCALHOST);
        local.shutdown().await;
        let error = runtime::start_with_options(
            0,
            AdapterKind::Reference,
            RuntimeOptions {
                network: NetworkOptions {
                    bind_ip: Ipv4Addr::UNSPECIFIED.into(),
                    join_token: None,
                },
                ..RuntimeOptions::default()
            },
        )
        .await
        .err()
        .expect("remote bind must fail");
        assert!(matches!(
            error,
            pilotage_session_host::error::HostError::Network(_)
        ));
    });
}

#[test]
fn configured_token_is_required_on_loopback_and_bad_first_frames_receive_no_state() {
    runtime().block_on(async {
        let host = start_host(Some(token())).await;
        let mut wrong = hello(Some(&JoinToken::from_hex(&"cd".repeat(32)).unwrap()));
        wrong.extend(media_attach());
        for bytes in [
            wrong,
            hello(None),
            lease(),
            media_attach(),
            vec![0x81, 0x80, 0x04],
        ] {
            let mut client = Client::connect(host.local_addr).await;
            client.send.write_all(&bytes).await.unwrap();
            client.assert_rejected_without_state().await;
        }
        host.shutdown().await;
    });
}

#[test]
fn a_valid_hello_keeps_pipelined_commands_and_credentials_do_not_grant_a_lease() {
    runtime().block_on(async {
        let host = start_host(Some(token())).await;
        let mut client = Client::connect(host.local_addr).await;
        client.send.write_all(&hello(Some(&token()))).await.unwrap();
        let welcome = client.welcome().await;
        assert!(
            welcome
                .scope_holders
                .iter()
                .all(|scope| scope.holder.is_none())
        );
        client.send.write_all(&lease()).await.unwrap();
        let granted = client.lease_response().await;
        assert!(granted.granted);
        client
            .connection
            .close(wtransport::VarInt::from_u32(0), b"done");
        host.shutdown().await;

        let host = start_host(Some(token())).await;
        let mut client = Client::connect(host.local_addr).await;
        let mut pipeline = hello(Some(&token()));
        pipeline.extend(lease());
        client.send.write_all(&pipeline).await.unwrap();
        client.welcome().await;
        assert!(client.lease_response().await.granted);
        client
            .connection
            .close(wtransport::VarInt::from_u32(0), b"done");
        host.shutdown().await;
    });
}

#[test]
fn unconfigured_loopback_accepts_an_empty_token_but_repeated_hello_is_rejected() {
    runtime().block_on(async {
        let host = start_host(None).await;
        let mut client = Client::connect(host.local_addr).await;
        client.send.write_all(&hello(None)).await.unwrap();
        client.welcome().await;
        client.send.write_all(&hello(None)).await.unwrap();
        assert_rejection(bounded(client.connection.closed()).await);
        host.shutdown().await;
    });
}

#[test]
fn missing_stream_and_partial_hello_expire_without_publishing_state() {
    runtime().block_on(async {
        let host = start_host(Some(token())).await;
        let no_stream = connect(host.local_addr).await;
        let mut partial = Client::connect(host.local_addr).await;
        partial.send.write_all(&[0x80]).await.unwrap();
        let (closed, ()) = tokio::join!(
            bounded(no_stream.closed()),
            partial.assert_rejected_without_state(),
        );
        assert_rejection(closed);
        assert!(no_stream.accept_uni().await.is_err());
        assert!(no_stream.receive_datagram().await.is_err());
        host.shutdown().await;
    });
}

#[test]
fn disconnect_releases_the_authenticated_lease_and_reconnect_requires_fresh_admission() {
    runtime().block_on(async {
        let host = start_host(Some(token())).await;
        let mut owner = Client::connect(host.local_addr).await;
        owner.send.write_all(&hello(Some(&token()))).await.unwrap();
        owner.welcome().await;
        owner.send.write_all(&lease()).await.unwrap();
        let old = owner.lease_response().await;
        assert!(old.granted);
        let mut observer = Client::connect(host.local_addr).await;
        observer
            .send
            .write_all(&hello(Some(&token())))
            .await
            .unwrap();
        observer.welcome().await;
        let mut events = bounded(observer.connection.accept_uni()).await.unwrap();
        owner
            .connection
            .close(wtransport::VarInt::from_u32(0), b"lost");
        wait_for_holder_link_loss(&mut events).await;
        observer.send.write_all(&lease()).await.unwrap();
        let new = observer.lease_response().await;
        assert!(new.granted);
        assert_ne!(old.generation, new.generation);
        let mut reconnect = Client::connect(host.local_addr).await;
        reconnect.send.write_all(&hello(None)).await.unwrap();
        reconnect.assert_rejected_without_state().await;
        observer
            .connection
            .close(wtransport::VarInt::from_u32(0), b"done");
        host.shutdown().await;
    });
}

fn assert_rejection(error: ConnectionError) {
    let ConnectionError::ApplicationClosed(close) = error else {
        panic!("expected application rejection, got {error}");
    };
    assert_eq!(
        close.code().into_inner(),
        u64::from(ADMISSION_REJECTED_CLOSE_CODE)
    );
    assert_eq!(close.reason(), b"admission rejected");
}

async fn wait_for_holder_link_loss(recv: &mut wtransport::RecvStream) {
    bounded(async {
        let mut tag = [0];
        recv.read_exact(&mut tag).await.unwrap();
        assert_eq!(tag, [1]);
        let mut pending = Vec::new();
        loop {
            let envelope = read_frame(recv, &mut pending).await;
            if let Some(wire::envelope::Payload::AuthorityEvent(event)) = envelope.payload
                && matches!(event.event,
                    Some(wire::authority_event::Event::WarningRaised(warning))
                        if warning.kind == wire::WarningKind::HolderLinkLost as i32)
            {
                return;
            }
        }
    })
    .await;
}
