#![allow(clippy::expect_used, clippy::panic)]

use super::TurnBasis;

#[test]
fn wire_bases_round_trip_and_unknown_fails_closed() {
    for basis in [TurnBasis::HeadingRate, TurnBasis::TrackRate] {
        assert_eq!(TurnBasis::from_u8(basis.to_u8()), basis);
    }
    for unknown in [2u8, 3, 100, 254, 255] {
        assert_eq!(TurnBasis::from_u8(unknown), TurnBasis::Unknown);
    }
    assert_eq!(
        TurnBasis::from_u8(TurnBasis::Unknown.to_u8()),
        TurnBasis::Unknown
    );
}

#[test]
fn body_rate_has_no_representation() {
    // The basis vocabulary is heading rate, track rate, or unknown —
    // there is no variant a feeder could use to label body yaw rate as
    // a turn indication, and the display never derives one.
    assert_eq!(TurnBasis::from_u8(0).label(), "HDG");
    assert_eq!(TurnBasis::from_u8(1).label(), "TRK");
    assert_eq!(TurnBasis::from_u8(9).label(), "REF");
}
