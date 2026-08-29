//! The deterministic run seed for one scenario and repetition.
//!
//! The partition constants keep training, promotion, and final qualification
//! on separate seed streams, so a candidate fitted to a training seed cannot
//! meet the same disturbance again on the run that decides what ships.

use super::MissionReference;
use crate::ScenarioSet;

pub(crate) fn derive_seed(
    fixed_seed: u64,
    set: ScenarioSet,
    scenario: &MissionReference,
    repetition: u32,
) -> u64 {
    let partition = match set {
        ScenarioSet::Training => 0x243f_6a88_85a3_08d3,
        ScenarioSet::Promotion => 0x1319_8a2e_0370_7344,
        ScenarioSet::FinalQualification => 0xa409_3822_299f_31d0,
    };
    let bytes = scenario.content_digest.as_bytes();
    let key = digest_word(bytes, 0)
        ^ digest_word(bytes, 8).rotate_left(13)
        ^ digest_word(bytes, 16).rotate_left(29)
        ^ digest_word(bytes, 24).rotate_left(47);
    split_mix(fixed_seed ^ partition ^ key ^ u64::from(repetition))
}

fn digest_word(bytes: &[u8; 32], start: usize) -> u64 {
    u64::from_le_bytes([
        bytes[start],
        bytes[start + 1],
        bytes[start + 2],
        bytes[start + 3],
        bytes[start + 4],
        bytes[start + 5],
        bytes[start + 6],
        bytes[start + 7],
    ])
}

fn split_mix(mut value: u64) -> u64 {
    value = value.wrapping_add(0x9e37_79b9_7f4a_7c15);
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}
