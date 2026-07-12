#![allow(clippy::expect_used, clippy::panic)]

use super::*;
use crate::condition::{AltFault, DisplayFault, DynFault, MiscompareFault, SystemNote};
use crate::event::FlightPhase;
use crate::profile::InhibitRule;

fn warn() -> AlertCondition {
    AlertCondition::Display(DisplayFault::RendererStalled)
}

fn attitude_warn() -> AlertCondition {
    AlertCondition::Miscompare(MiscompareFault::Attitude)
}

fn caution() -> AlertCondition {
    AlertCondition::Altitude(AltFault::ReferenceLost)
}

fn caution2() -> AlertCondition {
    AlertCondition::Miscompare(MiscompareFault::Airspeed)
}

fn advisory() -> AlertCondition {
    AlertCondition::TurnSlip(DynFault::TurnRateInvalid)
}

fn status() -> AlertCondition {
    AlertCondition::System(SystemNote::DatabaseStale)
}

fn maintenance() -> AlertCondition {
    AlertCondition::System(SystemNote::MaintenanceRequired)
}

/// Deadlines long enough that escalation never fires inside a test that
/// does not want it.
fn perm_profile() -> AlertProfile {
    AlertProfile::new(1_000_000, 1_000_000, 1_000_000, &[]).expect("valid profile")
}

fn ctx() -> AlertContext {
    AlertContext::default()
}

fn find(out: &AlertOutput, id: AlertId) -> Option<&ActiveAlert> {
    out.active().iter().find(|a| a.id == id)
}

fn state_of(out: &AlertOutput, id: AlertId) -> Option<AlertState> {
    find(out, id).map(|a| a.state)
}

#[test]
fn determinism_is_byte_identical() {
    let profile = perm_profile();
    let script: &[(&[AlertEvent], u64)] = &[
        (
            &[AlertEvent::Assert(caution()), AlertEvent::Assert(warn())],
            0,
        ),
        (&[AlertEvent::Acknowledge(warn().id())], 50),
        (&[AlertEvent::Clear(caution())], 100),
        (&[AlertEvent::Assert(advisory())], 150),
    ];
    let mut a = AlertManager::new();
    let mut b = AlertManager::new();
    for &(events, now) in script {
        let out_a = a.step(&profile, events, ctx(), now);
        let out_b = b.step(&profile, events, ctx(), now);
        assert_eq!(out_a, out_b);
    }
}

#[test]
fn simultaneous_severities_order_and_aural() {
    let profile = perm_profile();
    let mut m = AlertManager::new();
    let out = m.step(
        &profile,
        &[
            AlertEvent::Assert(advisory()),
            AlertEvent::Assert(warn()),
            AlertEvent::Assert(status()),
            AlertEvent::Assert(caution()),
            AlertEvent::Assert(maintenance()),
        ],
        ctx(),
        0,
    );
    let order = [
        out.active()[0].class,
        out.active()[1].class,
        out.active()[2].class,
        out.active()[3].class,
        out.active()[4].class,
    ];
    assert_eq!(
        order,
        [
            AlertClass::Warning,
            AlertClass::Caution,
            AlertClass::Advisory,
            AlertClass::Status,
            AlertClass::Maintenance,
        ]
    );
    assert_eq!(out.aural(), AuralToken::ContinuousTone);
}

#[test]
fn tie_break_by_ascending_id() {
    let profile = perm_profile();
    let mut m = AlertManager::new();
    // Two cautions; caution() is 0x0101, caution2() is 0x0402.
    let out = m.step(
        &profile,
        &[
            AlertEvent::Assert(caution2()),
            AlertEvent::Assert(caution()),
        ],
        ctx(),
        0,
    );
    assert_eq!(out.active()[0].id, caution().id());
    assert_eq!(out.active()[1].id, caution2().id());
}

#[test]
fn duplicate_assert_collapses() {
    let profile = perm_profile();
    let mut m = AlertManager::new();
    let out = m.step(
        &profile,
        &[
            AlertEvent::Assert(caution()),
            AlertEvent::Assert(caution()),
            AlertEvent::Assert(caution()),
        ],
        ctx(),
        0,
    );
    assert_eq!(out.active().len(), 1);
}

#[test]
fn acknowledge_keeps_condition_and_silences() {
    let profile = perm_profile();
    let mut m = AlertManager::new();
    m.step(&profile, &[AlertEvent::Assert(warn())], ctx(), 0);
    let out = m.step(&profile, &[AlertEvent::Acknowledge(warn().id())], ctx(), 10);
    assert_eq!(state_of(&out, warn().id()), Some(AlertState::Acknowledged));
    assert_eq!(out.aural(), AuralToken::Silent);
    // The condition is still present: not cleared by acknowledgement.
    assert_eq!(out.active().len(), 1);
}

#[test]
fn latched_warning_persists_until_ack() {
    let profile = perm_profile();
    let mut m = AlertManager::new();
    m.step(&profile, &[AlertEvent::Assert(warn())], ctx(), 0);
    // Condition clears while unacknowledged: warning latches, aural stops.
    let out = m.step(&profile, &[AlertEvent::Clear(warn())], ctx(), 10);
    assert_eq!(
        state_of(&out, warn().id()),
        Some(AlertState::LatchedCleared)
    );
    assert_eq!(out.aural(), AuralToken::Silent);
    // Acknowledgement of a cleared latched alert removes it.
    let out = m.step(&profile, &[AlertEvent::Acknowledge(warn().id())], ctx(), 20);
    assert!(find(&out, warn().id()).is_none());
}

#[test]
fn non_latching_advisory_self_clears() {
    let profile = perm_profile();
    let mut m = AlertManager::new();
    m.step(&profile, &[AlertEvent::Assert(advisory())], ctx(), 0);
    let out = m.step(&profile, &[AlertEvent::Clear(advisory())], ctx(), 10);
    assert!(find(&out, advisory().id()).is_none());
}

#[test]
fn recovery_re_alerts_after_clear() {
    let profile = perm_profile();
    let mut m = AlertManager::new();
    m.step(&profile, &[AlertEvent::Assert(caution())], ctx(), 0);
    let cleared = m.step(&profile, &[AlertEvent::Clear(caution())], ctx(), 10);
    assert_eq!(
        state_of(&cleared, caution().id()),
        Some(AlertState::LatchedCleared)
    );
    // Condition returns: re-alert, aural sounds again, not stuck.
    let out = m.step(&profile, &[AlertEvent::Assert(caution())], ctx(), 20);
    assert_eq!(state_of(&out, caution().id()), Some(AlertState::Active));
    assert_eq!(out.aural(), AuralToken::TripleChime);
}

#[test]
fn inhibit_entry_and_exit() {
    let rule = InhibitRule {
        id: caution().id(),
        phase: FlightPhase::Approach,
    };
    let profile = AlertProfile::new(1_000_000, 1_000_000, 1_000_000, &[rule]).expect("valid");
    let cruise = AlertContext {
        phase: FlightPhase::Cruise,
        ..AlertContext::default()
    };
    let approach = AlertContext {
        phase: FlightPhase::Approach,
        ..AlertContext::default()
    };
    let mut m = AlertManager::new();
    let out = m.step(&profile, &[AlertEvent::Assert(caution())], cruise, 0);
    assert!(!find(&out, caution().id()).expect("present").inhibited);
    assert_eq!(out.aural(), AuralToken::TripleChime);

    // Enter the inhibiting phase: flagged inhibited, silenced.
    let out = m.step(&profile, &[], approach, 10);
    assert!(find(&out, caution().id()).expect("present").inhibited);
    assert_eq!(out.aural(), AuralToken::Silent);

    // Exit the inhibiting phase: no longer inhibited.
    let out = m.step(&profile, &[], cruise, 20);
    assert!(!find(&out, caution().id()).expect("present").inhibited);
}

#[test]
fn escalation_boundary_at_and_before_deadline() {
    let profile = AlertProfile::new(1_000_000, 1_000, 1_000_000, &[]).expect("valid");
    let mut m = AlertManager::new();
    // Onset chimes once.
    let out = m.step(&profile, &[AlertEvent::Assert(caution())], ctx(), 0);
    assert_eq!(out.aural(), AuralToken::TripleChime);
    // One millisecond before the deadline: still silent.
    let out = m.step(&profile, &[], ctx(), 999);
    assert_eq!(out.aural(), AuralToken::Silent);
    // At the deadline: re-chimes.
    let out = m.step(&profile, &[], ctx(), 1_000);
    assert_eq!(out.aural(), AuralToken::TripleChime);
}

#[test]
fn aural_preemption_then_resume() {
    let profile = perm_profile();
    let mut m = AlertManager::new();
    // Caution and warning onset together: warning preempts, caution stays
    // pending.
    let out = m.step(
        &profile,
        &[AlertEvent::Assert(caution()), AlertEvent::Assert(warn())],
        ctx(),
        0,
    );
    assert_eq!(out.aural(), AuralToken::ContinuousTone);
    // Acknowledge the warning: the queued caution chime resumes.
    let out = m.step(&profile, &[AlertEvent::Acknowledge(warn().id())], ctx(), 10);
    assert_eq!(out.aural(), AuralToken::TripleChime);
}

fn fill_with_cautions(m: &mut AlertManager, profile: &AlertProfile) {
    let mut events = [AlertEvent::AcknowledgeAll; MAX_ACTIVE_ALERTS];
    for (i, e) in events.iter_mut().enumerate() {
        *e = AlertEvent::Assert(AlertCondition::FrameMismatch {
            code: (i as u8) + 1,
        });
    }
    let out = m.step(profile, &events, ctx(), 0);
    assert_eq!(out.active().len(), MAX_ACTIVE_ALERTS);
}

#[test]
fn overflow_evicts_lowest_for_warning() {
    let profile = perm_profile();
    let mut m = AlertManager::new();
    fill_with_cautions(&mut m, &profile);
    let out = m.step(&profile, &[AlertEvent::Assert(warn())], ctx(), 10);
    assert!(out.overflow());
    assert_eq!(out.overflow_dropped(), 1);
    assert_eq!(out.active().len(), MAX_ACTIVE_ALERTS);
    // The warning is present; the highest-id (lowest-priority) caution went.
    assert!(find(&out, warn().id()).is_some());
    let dropped = AlertCondition::FrameMismatch {
        code: MAX_ACTIVE_ALERTS as u8,
    };
    assert!(find(&out, dropped.id()).is_none());
}

#[test]
fn overflow_rejects_lower_priority_newcomer() {
    let profile = perm_profile();
    let mut m = AlertManager::new();
    fill_with_cautions(&mut m, &profile);
    let out = m.step(&profile, &[AlertEvent::Assert(advisory())], ctx(), 10);
    assert!(out.overflow());
    assert_eq!(out.overflow_dropped(), 1);
    // A lower-priority newcomer is dropped; no caution is displaced.
    assert!(find(&out, advisory().id()).is_none());
    assert_eq!(out.active().len(), MAX_ACTIVE_ALERTS);
    let first = AlertCondition::FrameMismatch { code: 1 };
    assert!(find(&out, first.id()).is_some());
}

#[test]
fn generation_wraps() {
    let profile = perm_profile();
    let mut m = AlertManager::new();
    m.generation = u32::MAX;
    let out = m.step(&profile, &[AlertEvent::Assert(caution())], ctx(), 0);
    assert_eq!(out.generation(), 0);
    assert_eq!(out.active()[0].generation, 0);
}

#[test]
fn manager_fault_preserves_primary_data_alert() {
    let profile = perm_profile();
    let mut m = AlertManager::new();
    let faulted = AlertContext {
        alerting_path_healthy: false,
        ..AlertContext::default()
    };
    let out = m.step(&profile, &[AlertEvent::Assert(attitude_warn())], faulted, 0);
    assert_eq!(out.health(), ManagerHealth::Faulted);
    // The primary-data warning is still present alongside the fault flag.
    assert_eq!(
        state_of(&out, attitude_warn().id()),
        Some(AlertState::Active)
    );
}

#[test]
fn unusual_attitude_declutter_retains_warning_and_caution() {
    let profile = perm_profile();
    let mut m = AlertManager::new();
    let declutter = AlertContext {
        declutter: true,
        ..AlertContext::default()
    };
    let out = m.step(
        &profile,
        &[
            AlertEvent::Assert(warn()),
            AlertEvent::Assert(caution()),
            AlertEvent::Assert(advisory()),
        ],
        declutter,
        0,
    );
    assert!(!find(&out, warn().id()).expect("present").decluttered);
    assert!(!find(&out, caution().id()).expect("present").decluttered);
    assert!(find(&out, advisory().id()).expect("present").decluttered);
    // The decluttered advisory does not sound; the warning still does.
    assert_eq!(out.aural(), AuralToken::ContinuousTone);
}

#[test]
fn acknowledge_all_silences_everything() {
    let profile = perm_profile();
    let mut m = AlertManager::new();
    m.step(
        &profile,
        &[AlertEvent::Assert(warn()), AlertEvent::Assert(caution())],
        ctx(),
        0,
    );
    let out = m.step(&profile, &[AlertEvent::AcknowledgeAll], ctx(), 10);
    assert_eq!(out.aural(), AuralToken::Silent);
    assert_eq!(state_of(&out, warn().id()), Some(AlertState::Acknowledged));
    assert_eq!(
        state_of(&out, caution().id()),
        Some(AlertState::Acknowledged)
    );
}

#[test]
fn transition_table_latching() {
    let profile = perm_profile();
    let mut m = AlertManager::new();
    let id = caution().id();

    // none -> Active
    let out = m.step(&profile, &[AlertEvent::Assert(caution())], ctx(), 0);
    assert_eq!(state_of(&out, id), Some(AlertState::Active));

    // Active -> Acknowledged
    let out = m.step(&profile, &[AlertEvent::Acknowledge(id)], ctx(), 1);
    assert_eq!(state_of(&out, id), Some(AlertState::Acknowledged));

    // Acknowledged + Clear -> removed (acknowledged and cleared)
    let out = m.step(&profile, &[AlertEvent::Clear(caution())], ctx(), 2);
    assert!(find(&out, id).is_none());

    // none -> Active -> LatchedCleared (clear while unacknowledged)
    m.step(&profile, &[AlertEvent::Assert(caution())], ctx(), 3);
    let out = m.step(&profile, &[AlertEvent::Clear(caution())], ctx(), 4);
    assert_eq!(state_of(&out, id), Some(AlertState::LatchedCleared));

    // LatchedCleared -> Active (recovery)
    let out = m.step(&profile, &[AlertEvent::Assert(caution())], ctx(), 5);
    assert_eq!(state_of(&out, id), Some(AlertState::Active));

    // Active -> LatchedCleared -> removed via acknowledgement
    m.step(&profile, &[AlertEvent::Clear(caution())], ctx(), 6);
    let out = m.step(&profile, &[AlertEvent::Acknowledge(id)], ctx(), 7);
    assert!(find(&out, id).is_none());
}

#[test]
fn transition_table_non_latching() {
    let profile = perm_profile();
    let mut m = AlertManager::new();
    let id = advisory().id();

    // none -> Active
    let out = m.step(&profile, &[AlertEvent::Assert(advisory())], ctx(), 0);
    assert_eq!(state_of(&out, id), Some(AlertState::Active));

    // Active -> Acknowledged (still present)
    let out = m.step(&profile, &[AlertEvent::Acknowledge(id)], ctx(), 1);
    assert_eq!(state_of(&out, id), Some(AlertState::Acknowledged));

    // Acknowledged + Clear -> removed
    let out = m.step(&profile, &[AlertEvent::Clear(advisory())], ctx(), 2);
    assert!(find(&out, id).is_none());

    // Active + Clear -> removed directly (self-clearing)
    m.step(&profile, &[AlertEvent::Assert(advisory())], ctx(), 3);
    let out = m.step(&profile, &[AlertEvent::Clear(advisory())], ctx(), 4);
    assert!(find(&out, id).is_none());
}
