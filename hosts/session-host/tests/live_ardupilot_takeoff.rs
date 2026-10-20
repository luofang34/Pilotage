//! Explicit takeoff through WebTransport and a real ArduPilot SITL process.
//!
//! Start a local ArduCopter SITL process with a UDP output to the address in
//! `PILOTAGE_ARDUPILOT_ADDR`. This test starts its own session host. Run with:
//!
//! ```text
//! PILOTAGE_ARDUPILOT_SITL=1 PILOTAGE_ARDUPILOT_PROFILE=simulation \
//! PILOTAGE_ARDUPILOT_ADDR=127.0.0.1:<port> \
//! cargo test -p pilotage-session-host --test live_ardupilot_takeoff \
//! -- --ignored --nocapture
//! ```
//!
//! The test requires a loopback simulator endpoint. It does not start SITL.
//! The cleanup action lands when airborne and then requests disarm.

#[cfg(test)]
#[path = "live_flight/client.rs"]
mod client;

#[test]
#[ignore = "requires an explicitly enabled local ArduCopter SITL process"]
fn a_client_observes_native_takeoff_and_disarm_through_the_host() {
    assert_eq!(
        std::env::var("PILOTAGE_ARDUPILOT_SITL").as_deref(),
        Ok("1"),
        "the live simulator test requires explicit opt-in"
    );
    assert_eq!(
        std::env::var("PILOTAGE_ARDUPILOT_PROFILE").as_deref(),
        Ok("simulation")
    );
    let endpoint = std::env::var("PILOTAGE_ARDUPILOT_ADDR")
        .expect("an explicit SITL endpoint is required")
        .parse::<std::net::SocketAddr>()
        .expect("SITL endpoint");
    assert!(endpoint.ip().is_loopback() && endpoint.port() != 0);
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_test_writer()
        .try_init()
        .ok();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("runtime");
    runtime.block_on(exercise());
}

#[cfg(test)]
async fn exercise() {
    use std::time::Duration;

    use pilotage_session_host::cli::AdapterKind;
    use pilotage_session_host::runtime;

    let host = runtime::start_with_options(
        0,
        AdapterKind::ArduPilot,
        runtime::RuntimeOptions::default(),
    )
    .await
    .expect("simulation host starts");
    let mut client = client::Client::connect(host.local_addr).await;
    let result = tokio::time::timeout(Duration::from_secs(120), client.takeoff_exercise()).await;
    let cleanup = tokio::time::timeout(Duration::from_secs(60), client.disarm_to_ground()).await;
    let shutdown = tokio::time::timeout(Duration::from_secs(10), host.shutdown()).await;
    assert!(shutdown.is_ok(), "host did not shut down");
    assert!(
        matches!(&cleanup, Ok(Ok(()))),
        "SITL cleanup failed: {cleanup:?}; takeoff result: {result:?}"
    );
    assert!(
        matches!(&result, Ok(Ok(()))),
        "takeoff was not observed complete: {result:?}"
    );
}
