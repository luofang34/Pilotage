use super::addressed_to_gcs;

#[test]
fn an_ack_addresses_this_gcs_by_its_identity_or_by_broadcast() {
    for (system, component) in [(0, 0), (255, 190), (255, 0), (0, 190)] {
        assert!(
            addressed_to_gcs(system, component),
            "({system}, {component})"
        );
    }
    for (system, component) in [(1, 0), (255, 1), (254, 190)] {
        assert!(
            !addressed_to_gcs(system, component),
            "({system}, {component})"
        );
    }
}
