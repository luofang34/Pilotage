use super::{ZoomCommand, ZoomUnit, wire};

#[test]
fn typed_zoom_units_and_directions_round_trip() {
    for command in [
        ZoomCommand::Absolute {
            unit: ZoomUnit::Ratio,
            value_micros: 2_500_000,
        },
        ZoomCommand::Absolute {
            unit: ZoomUnit::HorizontalFovRadians,
            value_micros: 900_000,
        },
        ZoomCommand::Step(-2),
        ZoomCommand::Continuous(0),
        ZoomCommand::Continuous(-1_000_000),
    ] {
        assert_eq!(
            ZoomCommand::try_from(wire::ZoomCommand::from(command)).ok(),
            Some(command)
        );
    }
}

#[test]
fn invalid_zoom_units_and_rates_are_rejected() {
    assert!(
        ZoomCommand::try_from(wire::ZoomCommand {
            operation: Some(wire::zoom_command::Operation::ContinuousMillionths(
                1_000_001
            )),
        })
        .is_err()
    );
    assert!(
        ZoomCommand::try_from(wire::ZoomCommand {
            operation: Some(wire::zoom_command::Operation::Absolute(
                wire::ZoomAbsoluteTarget {
                    unit: wire::ZoomUnit::Unspecified as i32,
                    value_micros: 1_000_000,
                }
            )),
        })
        .is_err()
    );
}
