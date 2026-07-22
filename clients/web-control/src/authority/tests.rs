#![allow(clippy::expect_used, clippy::panic)]

use super::{
    AuthorityDisposition, AuthorityEvent, AuthorityScope, AuthorityTable, is_fresh_generation,
};
use crate::plan::LeaseAction;

#[test]
fn every_scope_rejects_a_grant_that_does_not_clear_its_fence() {
    for scope in AuthorityScope::ALL {
        let mut table = AuthorityTable::default();
        table.apply(scope, AuthorityEvent::LeaseGranted { generation: 7 });
        table.apply(scope, AuthorityEvent::LeaseReleased { generation: 9 });
        assert_eq!(
            table.apply(scope, AuthorityEvent::LeaseGranted { generation: 8 }),
            AuthorityDisposition::Stale,
            "{scope:?} accepts no sub-fence grant"
        );
        assert!(!table.state(scope).granted());
        assert_eq!(table.state(scope).generation(), 7);
        assert_eq!(table.state(scope).fence(), 9);
        assert_eq!(
            table.apply(scope, AuthorityEvent::LeaseGranted { generation: 10 }),
            AuthorityDisposition::Applied
        );
        assert!(table.state(scope).granted());
        assert_eq!(table.state(scope).generation(), 10);
    }
}

#[test]
fn terminal_denial_survives_a_later_grant_until_a_new_session() {
    let mut table = AuthorityTable::default();
    table.apply(AuthorityScope::Motion, AuthorityEvent::LeaseDenied);
    assert!(table.state(AuthorityScope::Motion).denied());
    assert_eq!(
        table.apply(
            AuthorityScope::Motion,
            AuthorityEvent::LeaseGranted { generation: 42 }
        ),
        AuthorityDisposition::Ignored
    );
    assert!(!table.state(AuthorityScope::Motion).granted());
    table.begin_session();
    assert_eq!(
        table.apply(
            AuthorityScope::Motion,
            AuthorityEvent::LeaseGranted { generation: 1 }
        ),
        AuthorityDisposition::Applied
    );
}

#[test]
fn modular_generation_order_accepts_wrap_and_rejects_ambiguity() {
    assert!(is_fresh_generation(8, 7));
    assert!(!is_fresh_generation(7, 7));
    assert!(!is_fresh_generation(5, 7));
    assert!(is_fresh_generation(0, u64::MAX));
    assert!(!is_fresh_generation(u64::MAX, 0));
    assert!(!is_fresh_generation(1_u64 << 63, 0));
}

#[test]
fn a_delayed_older_fence_event_cannot_revoke_fresh_authority() {
    let mut table = AuthorityTable::default();
    let gimbal = AuthorityScope::Gimbal;
    table.apply(gimbal, AuthorityEvent::LeaseGranted { generation: 7 });
    table.apply(gimbal, AuthorityEvent::LeaseReleased { generation: 7 });
    table.apply(gimbal, AuthorityEvent::LeaseGranted { generation: 8 });
    assert_eq!(
        table.apply(gimbal, AuthorityEvent::LeaseReleased { generation: 7 }),
        AuthorityDisposition::Ignored
    );
    assert!(table.state(gimbal).granted());
    assert_eq!(table.state(gimbal).generation(), 8);
    assert_eq!(table.state(gimbal).fence(), 7);
}

#[test]
fn needs_arm_changes_only_on_authoritative_enactment_signals() {
    let mut table = AuthorityTable::default();
    let motion = AuthorityScope::Motion;
    table.apply(motion, AuthorityEvent::UplinkIdle);
    assert!(table.state(motion).needs_arm());
    table.apply(
        motion,
        AuthorityEvent::ActionResult {
            action: 1,
            accepted: false,
        },
    );
    assert!(table.state(motion).needs_arm());
    table.apply(
        motion,
        AuthorityEvent::ActionResult {
            action: 3,
            accepted: true,
        },
    );
    assert!(table.state(motion).needs_arm());
    table.apply(
        motion,
        AuthorityEvent::ActionResult {
            action: 1,
            accepted: true,
        },
    );
    assert!(!table.state(motion).needs_arm());
    table.apply(
        motion,
        AuthorityEvent::ActionResult {
            action: 2,
            accepted: true,
        },
    );
    assert!(table.state(motion).needs_arm());
}

#[test]
fn lease_plans_are_debounced_and_release_waits_for_acknowledgement() {
    let mut table = AuthorityTable::default();
    let motion = AuthorityScope::Motion;
    assert_eq!(table.plan(motion, true, 1000.0), Some(LeaseAction::Request));
    assert_eq!(table.plan(motion, true, 1100.0), None);
    assert_eq!(table.plan(motion, true, 1250.0), Some(LeaseAction::Request));
    table.apply(motion, AuthorityEvent::LeaseGranted { generation: 1 });
    assert_eq!(
        table.plan(motion, false, 1300.0),
        Some(LeaseAction::Release)
    );
    assert_eq!(table.plan(motion, false, 1400.0), None);
    assert!(table.state(motion).granted());
    table.apply(motion, AuthorityEvent::LeaseReleased { generation: 1 });
    assert!(!table.state(motion).granted());
    assert_eq!(table.plan(motion, false, 1600.0), None);
}

#[test]
fn recovery_requires_the_current_granted_motion_generation() {
    let mut table = AuthorityTable::default();
    let motion = AuthorityScope::Motion;
    table.apply(motion, AuthorityEvent::LeaseGranted { generation: 4 });
    table.apply(motion, AuthorityEvent::Revoked { generation: 4 });
    table.apply(motion, AuthorityEvent::LeaseGranted { generation: 5 });
    assert!(!table.state(motion).recovered());
    assert_eq!(
        table.apply(motion, AuthorityEvent::LinkLossCleared { generation: 4 }),
        AuthorityDisposition::Ignored
    );
    assert_eq!(
        table.apply(motion, AuthorityEvent::LinkLossCleared { generation: 5 }),
        AuthorityDisposition::Applied
    );
    assert!(table.state(motion).recovered());
}
