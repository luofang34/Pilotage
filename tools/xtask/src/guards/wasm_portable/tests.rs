use super::forbidden_in_tree;

#[test]
fn a_socket_runtime_in_the_tree_is_named() {
    let listing = "pilotage-mavlink v0.1.0 (/repo/crates/pilotage-mavlink)\n\
                   thiserror v2.0.12\n\
                   tokio v1.52.3\n\
                   mio v1.2.1\n\
                   tokio v1.52.3 (*)\n";
    assert_eq!(forbidden_in_tree(listing), ["tokio", "mio"]);
}

#[test]
fn a_crate_whose_name_only_contains_a_forbidden_name_passes() {
    let listing = "pilotage-mavlink v0.1.0\n\
                   tokio-util-free v0.1.0\n\
                   socket2-free v0.1.0\n";
    assert!(forbidden_in_tree(listing).is_empty());
}
