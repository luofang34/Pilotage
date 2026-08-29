#![allow(clippy::expect_used, clippy::panic)]

use sha2::{Digest as ShaDigest, Sha256};

use super::*;

const GOLDEN_RUN_SEED: u64 = 0x1112_1314_1516_1718;
const GOLDEN_CONDITION_BYTES: [u8; 32] = [
    0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25,
    26, 27, 28, 29, 30, 31,
];

fn policy(fraction_basis_points: u16, decision_interval_samples: u32) -> CommandLossPolicy {
    CommandLossPolicy::SeededZeroOrderHold {
        fraction_basis_points,
        decision_interval_samples,
    }
}

fn identity(
    run_seed: u64,
    interval_epoch: u64,
    interval_index: u64,
    first_eligible_global_sample_sequence: u64,
) -> CommandHoldIntervalIdentity {
    CommandHoldIntervalIdentity::new(
        Digest::from_bytes(GOLDEN_CONDITION_BYTES),
        run_seed,
        interval_epoch,
        interval_index,
        first_eligible_global_sample_sequence,
    )
    .expect("interval identity")
}

#[test]
fn permutation_domain_and_u64_little_endian_encoding_have_a_golden_value() {
    let interval_epoch = 0x2122_2324_2526_2728_u64;
    let interval_index = 0x3132_3334_3536_3738_u64;
    let first_eligible_global_sample_sequence = 0x4142_4344_4546_4748_u64;
    let cursor = 0x5152_5354_5556_5758_u64;
    let input = [
        COMMAND_HOLD_DOMAIN,
        GOLDEN_CONDITION_BYTES.as_slice(),
        GOLDEN_RUN_SEED.to_le_bytes().as_slice(),
        interval_epoch.to_le_bytes().as_slice(),
        interval_index.to_le_bytes().as_slice(),
        first_eligible_global_sample_sequence
            .to_le_bytes()
            .as_slice(),
        cursor.to_le_bytes().as_slice(),
    ]
    .concat();
    let digest = Sha256::digest(input);

    assert_eq!(COMMAND_HOLD_DOMAIN, b"pilotage-command-hold-v1");
    assert_eq!(
        digest.as_slice(),
        &[
            0x22, 0xd1, 0x68, 0xe2, 0x28, 0xb0, 0xa4, 0x48, 0x63, 0x9b, 0x56, 0x27, 0x06, 0x44,
            0xf7, 0xd3, 0x7e, 0x49, 0x2d, 0xef, 0xc8, 0x5c, 0x69, 0xc9, 0xce, 0x7b, 0x66, 0x56,
            0x30, 0x98, 0x48, 0x5c,
        ]
    );
    let identity = identity(
        GOLDEN_RUN_SEED,
        interval_epoch,
        interval_index,
        first_eligible_global_sample_sequence,
    );
    assert_eq!(
        identity.digest().to_string(),
        "94ab1093b990a952b30ec29395a88314347304a5b21ed170c862d2725d99bd6c"
    );
    assert_eq!(
        permutation_value(identity, cursor),
        5_234_502_356_555_059_490
    );
}

#[test]
fn a_platform_width_cursor_cannot_change_the_hold_schedule() {
    // A 32-bit host writes a `usize` cursor in four bytes. An implementation
    // that encoded the platform type would take a shorter preimage and give a
    // different swap, so the schedule would depend on the build target.
    let identity = identity(GOLDEN_RUN_SEED, 0, 0, 1_001);
    let cursor = 99_u64;
    let mut narrow = Sha256::new();
    identity.update_hasher(&mut narrow);
    narrow.update(u32::try_from(cursor).expect("narrow cursor").to_le_bytes());
    let narrow = narrow.finalize();

    assert_ne!(
        permutation_value(identity, cursor).to_le_bytes(),
        [
            narrow[0], narrow[1], narrow[2], narrow[3], narrow[4], narrow[5], narrow[6], narrow[7],
        ]
    );
}

#[test]
fn prime_precedes_interval_zero_and_each_complete_interval_has_exact_holds() {
    let value = policy(100, 100);
    let decisions = value
        .decisions_for_interval(identity(GOLDEN_RUN_SEED, 0, 0, 1_001))
        .expect("valid hold policy");
    let held = decisions
        .iter()
        .enumerate()
        .filter_map(|(index, hold)| hold.then_some(index))
        .collect::<Vec<_>>();

    assert_eq!(decisions.len(), 100);
    assert_eq!(value.prime_action(), CommandHoldAction::Accept);
    assert_eq!(held, vec![45]);
    assert_eq!(
        usize::try_from(value.exact_hold_count().expect("hold count")).expect("count"),
        held.len()
    );
}

#[test]
fn the_same_seed_produces_the_same_applied_hold_schedule() {
    let value = policy(1_000, 100);
    let first = value
        .decisions_for_interval(identity(5, 11, 7, 901))
        .expect("first schedule");
    let repeated = value
        .decisions_for_interval(identity(5, 11, 7, 901))
        .expect("repeated schedule");
    let changed_run = value
        .decisions_for_interval(identity(6, 11, 7, 901))
        .expect("changed run schedule");
    let changed_start = value
        .decisions_for_interval(identity(5, 11, 7, 902))
        .expect("changed interval start");
    let changed_epoch = value
        .decisions_for_interval(identity(5, 12, 7, 901))
        .expect("changed interval epoch");
    let changed_index = value
        .decisions_for_interval(identity(5, 11, 8, 901))
        .expect("changed interval index");
    let changed_condition = value
        .decisions_for_interval(
            CommandHoldIntervalIdentity::new(Digest::from_bytes([4; 32]), 5, 11, 7, 901)
                .expect("changed condition identity"),
        )
        .expect("changed condition schedule");

    assert_eq!(first, repeated);
    assert_ne!(first, changed_run);
    assert_ne!(first, changed_start);
    assert_ne!(first, changed_epoch);
    assert_ne!(first, changed_index);
    assert_ne!(first, changed_condition);
    assert_eq!(first.len(), 100);
    assert_eq!(first.iter().filter(|hold| **hold).count(), 10);
}

#[test]
fn hold_policy_rejects_an_inexact_count_and_an_unbounded_request() {
    assert!(policy(0, 100).validate().is_err());
    assert!(policy(1, 100).validate().is_err());
    assert!(policy(100, 0).validate().is_err());
    assert!(policy(100, 10_001).validate().is_err());
    assert!(policy(1_001, 10_000).validate().is_err());
    assert!(matches!(
        policy(1, 100).validate(),
        Err(ValidationError::InvalidRelation { .. })
    ));
    assert!(policy(u16::MAX, u32::MAX).exact_hold_count().is_err());
    assert_eq!(
        CommandLossPolicy::None {}
            .exact_hold_count()
            .expect("nominal count"),
        0
    );
}

#[test]
fn authority_scale_holds_the_closed_basis_point_bound() {
    for scale in [4_999, 15_001] {
        let value = ActuatorCondition {
            authority_scale_basis_points: scale,
            command_loss: CommandLossPolicy::None {},
        };
        assert!(matches!(
            value.validate(),
            Err(ValidationError::OutOfRange { .. })
        ));
    }
    for scale in [5_000, 10_000, 15_000] {
        let value = ActuatorCondition {
            authority_scale_basis_points: scale,
            command_loss: CommandLossPolicy::None {},
        };
        value.validate().expect("bounded authority");
    }
    assert!(ActuatorCondition::nominal().has_nominal_authority());
    assert!(
        (ActuatorCondition {
            authority_scale_basis_points: 8_000,
            command_loss: CommandLossPolicy::None {},
        }
        .authority_scale()
            - 0.8)
            .abs()
            < 1e-12
    );
}

#[test]
fn only_a_non_nominal_request_needs_a_capability() {
    assert!(
        ActuatorCondition::nominal()
            .required_capabilities()
            .is_empty()
    );
    assert_eq!(
        ActuatorCondition {
            authority_scale_basis_points: 12_000,
            command_loss: policy(100, 100),
        }
        .required_capabilities(),
        vec![
            BackendCapability::ActuatorAuthority,
            BackendCapability::CommandHold
        ]
    );
}

#[test]
fn a_zero_condition_digest_is_not_an_interval_identity() {
    assert!(CommandHoldIntervalIdentity::new(Digest::from_bytes([0; 32]), 1, 0, 0, 2).is_err());
}

#[test]
fn a_selected_first_hold_accepts_the_first_safe_command() {
    assert_eq!(
        CommandLossPolicy::action(true, false),
        CommandHoldAction::Accept
    );
    assert_eq!(
        CommandLossPolicy::action(true, true),
        CommandHoldAction::HoldLastAccepted
    );
    assert_eq!(
        CommandLossPolicy::action(false, true),
        CommandHoldAction::Accept
    );
}
