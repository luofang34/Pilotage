//! Increment-0 acceptance: client core and test host exchange fixture
//! sessions (ADR-0002, ADR-0008, ADR-0010, ADR-0012).
//!
//! Every test here drives the public [`increment_zero_script`] through the
//! public [`ScriptedSession`] harness and asserts one facet of conformance:
//! the exact ordered authority audit trail, the exact frame verdicts, the
//! golden adapter trajectory, wire round-trips for both message families,
//! deterministic replay, and snapshot/restore convergence. It touches no
//! crate internals.

#![allow(clippy::expect_used, clippy::panic)]

use core::time::Duration;

use pilotage_adapter_api::{Disposition, VehicleAdapter};
use pilotage_authority::{
    AuthorityClass, AuthorityEffect, FrameVerdict, LinkState, OverrideReason,
};
use pilotage_conformance::{
    FrameOutcome, OPERATOR_A, OPERATOR_B, OPERATOR_C, SEED, Script, ScriptStep, ScriptedSession,
    SessionEvent, TrajectoryCheckpoint, VEHICLE, aged_frame_is_stale, authority_event_roundtrips,
    control_frame_roundtrips, increment_zero_checkpoints, increment_zero_script,
};
use pilotage_protocol::{
    ControlPayload, Generation, LogicalAxisId, ScopeId, ScopedControlFrame, SequenceNum, SessionId,
    VehicleId,
};
use pilotage_timing::{Freshness, MonoTimestamp, StalenessPolicy};

fn motion() -> ScopeId {
    ScopeId::new("vehicle.motion")
}

fn camera() -> ScopeId {
    ScopeId::new("vehicle.camera")
}

/// The exact ordered authority effects the increment-0 session emits, in the
/// order the host observes and persists them (ADR-0012 audit trail).
fn expected_authority_effects() -> Vec<AuthorityEffect> {
    vec![
        AuthorityEffect::ScopeRegistered {
            vehicle: VEHICLE,
            scope: motion(),
        },
        AuthorityEffect::ScopeRegistered {
            vehicle: VEHICLE,
            scope: camera(),
        },
        AuthorityEffect::ScopeLeaseGranted {
            vehicle: VEHICLE,
            scope: motion(),
            holder: OPERATOR_A,
            generation: Generation::new(1),
        },
        AuthorityEffect::ScopeLeaseGranted {
            vehicle: VEHICLE,
            scope: camera(),
            holder: OPERATOR_B,
            generation: Generation::new(1),
        },
        AuthorityEffect::ScopeTransferOffered {
            vehicle: VEHICLE,
            scope: motion(),
            from: OPERATOR_A,
            to: OPERATOR_B,
            generation: Generation::new(1),
            expires_at: MonoTimestamp::from_nanos(10_000_000_000),
        },
        AuthorityEffect::ScopeTransferCommitted {
            vehicle: VEHICLE,
            scope: motion(),
            from: OPERATOR_A,
            to: OPERATOR_B,
            generation: Generation::new(2),
        },
        AuthorityEffect::LinkStateChanged {
            vehicle: VEHICLE,
            scope: motion(),
            principal: OPERATOR_B,
            state: LinkState::Nominal,
        },
        AuthorityEffect::LinkStateChanged {
            vehicle: VEHICLE,
            scope: motion(),
            principal: OPERATOR_A,
            state: LinkState::Nominal,
        },
        AuthorityEffect::EmergencyOverrideApplied {
            vehicle: VEHICLE,
            scope: motion(),
            previous_holder: Some(OPERATOR_B),
            holder: OPERATOR_C,
            authority_class: AuthorityClass::Supervisor,
            reason: OverrideReason::new("range safety takeover"),
            generation: Generation::new(3),
        },
        AuthorityEffect::LinkStateChanged {
            vehicle: VEHICLE,
            scope: motion(),
            principal: OPERATOR_C,
            state: LinkState::Lost,
        },
        AuthorityEffect::HolderLinkLost {
            vehicle: VEHICLE,
            scope: motion(),
            lost_holder: OPERATOR_C,
            generation: Generation::new(4),
        },
    ]
}

fn authority_effects(events: &[SessionEvent]) -> Vec<AuthorityEffect> {
    events
        .iter()
        .filter_map(|event| event.as_authority().cloned())
        .collect()
}

fn frame_outcomes(events: &[SessionEvent]) -> Vec<FrameOutcome> {
    events
        .iter()
        .filter_map(|event| event.as_frame().cloned())
        .collect()
}

#[test]
fn authority_effect_list_is_exact_and_ordered() {
    let (events, _adapter) = ScriptedSession::run(&increment_zero_script());
    assert_eq!(authority_effects(&events), expected_authority_effects());
}

#[test]
fn frame_verdicts_fence_stale_generations() {
    let (events, _adapter) = ScriptedSession::run(&increment_zero_script());
    let outcomes = frame_outcomes(&events);

    // A drives at gen 1 (accepted), A's post-handover gen-1 frame is fenced,
    // B drives at gen 2 (accepted), C's pre-override gen-2 frame is fenced,
    // C drives at gen 3 (accepted).
    let expected = vec![
        FrameOutcome {
            sequence: SequenceNum::new(1),
            verdict: FrameVerdict::Accepted,
            disposition: Some(Disposition::Accepted),
            applied_tick: Some(pilotage_timing::SimTick::new(0)),
        },
        FrameOutcome {
            sequence: SequenceNum::new(2),
            verdict: FrameVerdict::RejectedStaleGeneration {
                current: Generation::new(2),
            },
            disposition: None,
            applied_tick: None,
        },
        FrameOutcome {
            sequence: SequenceNum::new(3),
            verdict: FrameVerdict::Accepted,
            disposition: Some(Disposition::Accepted),
            applied_tick: Some(pilotage_timing::SimTick::new(10)),
        },
        FrameOutcome {
            sequence: SequenceNum::new(4),
            verdict: FrameVerdict::RejectedStaleGeneration {
                current: Generation::new(3),
            },
            disposition: None,
            applied_tick: None,
        },
        FrameOutcome {
            sequence: SequenceNum::new(5),
            verdict: FrameVerdict::Accepted,
            disposition: Some(Disposition::Accepted),
            applied_tick: Some(pilotage_timing::SimTick::new(20)),
        },
    ];
    assert_eq!(outcomes, expected);
}

/// Replays the script, capturing telemetry at each of the four stepped
/// phases so it can be matched against the golden checkpoints.
fn captured_checkpoints() -> Vec<TrajectoryCheckpoint> {
    let script = increment_zero_script();
    let mut session = ScriptedSession::new(script.vehicle, script.seed);
    let labels = increment_zero_checkpoints().map(|c| c.label);
    let mut captured = Vec::new();
    let mut label_iter = labels.into_iter();
    for step in &script.steps {
        session.apply(step);
        if matches!(step, ScriptStep::Step(_)) {
            let label = label_iter.next().expect("one label per stepped phase");
            let mut adapter = session.adapter().clone();
            captured.push(
                TrajectoryCheckpoint::capture(label, &mut adapter)
                    .expect("reference telemetry is complete"),
            );
        }
    }
    captured
}

#[test]
fn trajectory_matches_golden_checkpoints_exactly() {
    let captured = captured_checkpoints();
    let golden = increment_zero_checkpoints();
    assert_eq!(captured.len(), golden.len());
    for (got, want) in captured.iter().zip(golden.iter()) {
        assert_eq!(got, want, "checkpoint {} drifted", want.label);
    }
}

#[test]
fn neutralize_checkpoint_decays_speed_below_override() {
    let golden = increment_zero_checkpoints();
    let override_speed = f64::from_bits(golden[2].speed_bits);
    let neutralize_speed = f64::from_bits(golden[3].speed_bits);
    assert!(
        neutralize_speed < override_speed,
        "link-loss neutralize speed {neutralize_speed} should decay below \
         override speed {override_speed}"
    );
    assert!(
        neutralize_speed > 0.0,
        "one decay window does not reach rest"
    );
}

#[test]
fn every_control_frame_roundtrips_through_the_wire() {
    let script = increment_zero_script();
    let mut count = 0_usize;
    for step in &script.steps {
        if let ScriptStep::Frame(frame) = step {
            control_frame_roundtrips(frame).expect("control frame must round-trip");
            count += 1;
        }
    }
    assert_eq!(count, 5, "the fixture routes five control frames");
}

#[test]
fn every_authority_event_roundtrips_through_the_wire() {
    let (events, _adapter) = ScriptedSession::run(&increment_zero_script());
    let effects = authority_effects(&events);
    assert!(!effects.is_empty());
    for effect in &effects {
        authority_event_roundtrips(effect).expect("authority event must round-trip");
    }
}

#[test]
fn replay_from_same_seed_is_deterministic() {
    let script = increment_zero_script();
    let (events_a, mut adapter_a) = ScriptedSession::run(&script);
    let (events_b, mut adapter_b) = ScriptedSession::run(&script);
    assert_eq!(events_a, events_b, "event logs must be identical");
    assert_eq!(
        adapter_a.sample_telemetry(),
        adapter_b.sample_telemetry(),
        "final trajectories must be identical"
    );
    assert_eq!(
        adapter_a.snapshot().expect("snapshot a"),
        adapter_b.snapshot().expect("snapshot b"),
        "final adapter state must be identical"
    );
}

#[test]
fn snapshot_restore_midsession_converges_with_uninterrupted_run() {
    let script = increment_zero_script();
    let (baseline_events, baseline_adapter) = ScriptedSession::run(&script);

    // Run to a mid-session point, snapshot the adapter, restore it into a
    // fresh session, and finish the remaining steps.
    let midpoint = script.steps.len() / 2;
    let mut session = ScriptedSession::new(script.vehicle, script.seed);
    for step in &script.steps[..midpoint] {
        session.apply(step);
    }
    let snapshot = session.adapter().snapshot().expect("mid-session snapshot");
    let restored = pilotage_adapter_reference::ReferenceAdapter::restore(&snapshot)
        .expect("restore from snapshot");
    session.replace_adapter(restored);
    for step in &script.steps[midpoint..] {
        session.apply(step);
    }
    let (resumed_events, resumed_adapter) = session.into_parts();

    assert_eq!(
        resumed_events, baseline_events,
        "resumed event log must converge with the uninterrupted run"
    );
    assert_eq!(
        resumed_adapter.snapshot().expect("resumed snapshot"),
        baseline_adapter.snapshot().expect("baseline snapshot"),
        "resumed trajectory must converge with the uninterrupted run"
    );
}

#[test]
fn staleness_policy_rejects_an_artificially_aged_frame() {
    // A frame at the current generation and correct holder is still rejected
    // if it arrives older than the policy's maximum control age (ADR-0009).
    let policy = StalenessPolicy::new(Duration::from_millis(50));
    let frame = ScopedControlFrame {
        session: SessionId::new(7),
        vehicle: VehicleId::new(1),
        scope: motion(),
        generation: Generation::new(1),
        sequence: SequenceNum::new(1),
        sampled_at: MonoTimestamp::from_nanos(0),
        profile_revision: 1,
        activation_revision: 0,
        payload: ControlPayload {
            axes: vec![(LogicalAxisId::new(2), 1.0)],
            edges: vec![],
        },
        intent: None,
        actions: vec![],
    };

    let fresh_now = MonoTimestamp::from_nanos(40_000_000);
    assert_eq!(
        aged_frame_is_stale(&frame, fresh_now, &policy),
        Freshness::Fresh,
        "a 40ms-old frame is within the 50ms budget"
    );

    let aged_now = MonoTimestamp::from_nanos(60_000_000);
    match aged_frame_is_stale(&frame, aged_now, &policy) {
        Freshness::Stale { age } => assert_eq!(age, Duration::from_millis(60)),
        Freshness::Fresh => panic!("a 60ms-old frame must be rejected as stale"),
    }
}

#[test]
fn accepted_frames_only_when_generation_current() {
    // Sanity that the harness applies frames to the adapter exactly when
    // authority accepts them: five routed frames, three accepted, so three
    // adapter dispositions are present and two are absent.
    let (events, _adapter) = ScriptedSession::run(&increment_zero_script());
    let outcomes = frame_outcomes(&events);
    let applied = outcomes.iter().filter(|o| o.disposition.is_some()).count();
    let fenced = outcomes.iter().filter(|o| o.disposition.is_none()).count();
    assert_eq!(applied, 3);
    assert_eq!(fenced, 2);
}

/// A minimal second fixture proving the harness is reusable beyond the one
/// scripted session: a bare grant-and-drive with no handover.
#[test]
fn harness_runs_a_minimal_grant_and_drive_script() {
    let steps = vec![
        ScriptStep::Command(pilotage_authority::AuthorityCommand::RegisterScope {
            vehicle: VEHICLE,
            scope: motion(),
        }),
        ScriptStep::Command(pilotage_authority::AuthorityCommand::Grant {
            vehicle: VEHICLE,
            scope: motion(),
            to: OPERATOR_A,
        }),
        ScriptStep::Frame(ScopedControlFrame {
            session: SessionId::new(1),
            vehicle: VEHICLE,
            scope: motion(),
            generation: Generation::new(1),
            sequence: SequenceNum::new(1),
            sampled_at: MonoTimestamp::from_nanos(0),
            profile_revision: 1,
            activation_revision: 0,
            payload: ControlPayload {
                axes: vec![(LogicalAxisId::new(2), 1.0)],
                edges: vec![],
            },
            intent: None,
            actions: vec![],
        }),
        ScriptStep::Step(5),
    ];
    let script = Script {
        vehicle: VEHICLE,
        seed: SEED,
        steps,
    };
    let (events, mut adapter) = ScriptedSession::run(&script);
    let outcomes = frame_outcomes(&events);
    assert_eq!(outcomes.len(), 1);
    assert_eq!(outcomes[0].verdict, FrameVerdict::Accepted);
    assert!(
        adapter.sample_telemetry().samples[0]
            .speed
            .expect("reference speed")
            > 0.0
    );
}
