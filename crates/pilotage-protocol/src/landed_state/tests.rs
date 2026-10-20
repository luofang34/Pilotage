use super::LandedState;

#[test]
fn an_unrecognized_state_cannot_assert_ground_or_flight() {
    for raw in [0, -1, 5, i32::MAX] {
        assert_eq!(LandedState::from_wire(raw), LandedState::Unknown);
    }
}

#[test]
fn detector_states_preserve_their_meaning_on_the_wire() {
    for state in [
        LandedState::Unknown,
        LandedState::OnGround,
        LandedState::InAir,
        LandedState::TakingOff,
        LandedState::Landing,
    ] {
        assert_eq!(LandedState::from_wire(state.to_wire() as i32), state);
    }
}
