use super::Selection;

#[test]
fn duplicate_events_do_not_retry_an_attachment_and_reenumeration_is_new() {
    let mut selection = Selection::default();
    assert_eq!(selection.observe(&[]), None);
    assert_eq!(selection.observe(&[12]), Some(12));
    for _ in 0..10 {
        assert_eq!(selection.observe(&[12]), None);
    }
    assert_eq!(selection.observe(&[]), None);
    assert_eq!(selection.observe(&[12]), None);
    assert_eq!(selection.observe(&[34]), Some(34));
}

#[test]
fn multiple_devices_require_selection_and_recovery_has_an_offline_boundary() {
    let mut selection = Selection::default();
    assert_eq!(selection.observe(&[12, 34]), None);
    assert_eq!(selection.observe(&[34, 12]), None);
    assert_eq!(selection.observe(&[12]), Some(12));
    let next = selection.observe(&[34]);
    selection.defer(next);
    assert_eq!(selection.take_deferred(), Some(34));
    assert_eq!(selection.take_deferred(), None);
    let next = selection.observe(&[56]);
    selection.defer(next);
    assert_eq!(selection.observe(&[]), None);
    assert_eq!(selection.take_deferred(), None);
}
