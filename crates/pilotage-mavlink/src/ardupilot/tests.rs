use super::ArduCopterMode;

#[test]
fn every_mode_number_round_trips() {
    for number in 0..=40 {
        assert_eq!(
            ArduCopterMode::from_custom_mode(number).custom_mode(),
            number
        );
    }
}

#[test]
fn guided_is_mode_four_and_unnamed_numbers_stay_unnamed() {
    assert_eq!(ArduCopterMode::from_custom_mode(4), ArduCopterMode::Guided);
    assert_eq!(ArduCopterMode::Guided.custom_mode(), 4);
    assert_eq!(
        ArduCopterMode::from_custom_mode(8),
        ArduCopterMode::Other(8)
    );
}
