use super::*;

fn needed(h: &mut Harness, sequence: u16, flags: u8) {
    let [low, high] = sequence.to_le_bytes();
    // Captured Mini 3 route and 31 08 payload report landing confirmation needed.
    let mut packet = vec![0x55, 15, 4, 0, 9, 2, low, high, 0, 3, 0x53, flags, 8, 0, 0];
    super::super::harness::seal_duml(&mut packet);
    super::super::flight::feed(h, &packet);
}

#[test]
fn fresh_confirmation_and_sustained_down_send_once_without_releasing_authority() {
    let mut h = override_control::landing();
    for tick in 0..35_u16 {
        let now = 30 + u64::from(tick) * 100;
        flying_mode(&mut h, tick + 4, 12);
        needed(&mut h, tick + 1, 0x31);
        h.step(now);
        assert_eq!(
            input(&mut h, u32::from(tick) + 2, now, [0.0, 0.0, 0.5, 0.0]),
            Disposition::Accepted
        );
        h.step(now + 1);
        let confirmations = commands(&h)
            .iter()
            .filter(|p| p.command_id() == 0x2a && p.payload() == [30])
            .count();
        assert_eq!(confirmations, usize::from(tick >= 20));
    }
    // Confirmation is a follow-up within landing, not a lease release or retry.
    input(&mut h, 38, 3432, [0.0; 4]);
    input(&mut h, 39, 3433, [0.2, 0.0, 0.0, 0.0]);
    h.step(3434);
    assert_eq!(commands(&h).last().unwrap().payload(), [14]);
}

#[test]
fn missing_stale_replayed_or_released_confirmation_hold_never_sends() {
    for case in 0..5 {
        let mut h = override_control::landing();
        for tick in 0..25_u16 {
            let now = 30 + u64::from(tick) * 100;
            flying_mode(&mut h, tick + 4, 12);
            if case != 0 {
                needed(
                    &mut h,
                    if case == 1 { 1 } else { tick + 1 },
                    if case == 2 { 0x11 } else { 0x31 },
                );
            }
            h.step(now);
            let value = if case == 3 && tick % 5 == 0 {
                [0.0; 4]
            } else {
                [0.0, 0.0, 0.5, 0.0]
            };
            if case != 4 || tick % 2 == 0 {
                input(&mut h, u32::from(tick) + 2, now, value);
            }
            h.step(now + 1);
        }
        assert!(
            !commands(&h)
                .iter()
                .any(|p| p.command_id() == 0x2a && p.payload() == [30]),
            "case {case}"
        );
    }
}
