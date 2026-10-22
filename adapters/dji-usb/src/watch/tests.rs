use super::{Duration, UsbEvents};

#[test]
fn subscription_is_armed_before_initial_snapshot_and_closes_without_a_device() {
    let events = UsbEvents::subscribe_blocking().expect("subscribe to IOKit");
    events
        .changed
        .recv_timeout(Duration::from_secs(1))
        .expect("initial snapshot notification");
    crate::discover_upper_blocking().expect("read the current snapshot");
    drop(events);
}

#[test]
fn access_acknowledges_only_the_exact_supported_role_request() {
    assert_eq!(crate::ffi::access_test(), 0);
}
