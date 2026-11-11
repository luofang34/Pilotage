use navigate_contract::MonotonicNanos;
use pilotage_geo::VerticalDatum;

use super::{
    DeclaredHeading, DeclaredHeight, DeclaredPosition, DeclaredSpeed, DeclaredStart,
    DeclaredStartError, SpeedReference,
};

fn statement() -> DeclaredStart {
    DeclaredStart {
        position: Some(DeclaredPosition {
            latitude_rad: 0.70,
            longitude_rad: -1.30,
            sigma_m: 30.0,
        }),
        height: Some(DeclaredHeight {
            height_m: 120.0,
            datum: VerticalDatum::Msl,
            sigma_m: 10.0,
        }),
        heading: Some(DeclaredHeading {
            heading_rad: 1.57,
            sigma_rad: 0.1,
        }),
        speed: Some(DeclaredSpeed {
            speed_mps: 0.0,
            reference: SpeedReference::GroundSpeed,
            sigma_mps: 0.5,
        }),
        declared_at: MonotonicNanos::from_nanos(5),
    }
}

#[test]
fn a_full_statement_and_an_empty_one_are_accepted() {
    assert_eq!(statement().validate(), Ok(()));
    let empty = DeclaredStart {
        position: None,
        height: None,
        heading: None,
        speed: None,
        declared_at: MonotonicNanos::from_nanos(0),
    };
    assert_eq!(empty.validate(), Ok(()));
}

#[test]
fn each_unusable_part_is_named() {
    let refused = |change: fn(&mut DeclaredStart)| {
        let mut start = statement();
        change(&mut start);
        start
            .validate()
            .map_err(|DeclaredStartError { field }| field)
    };
    assert_eq!(
        refused(|s| s
            .position
            .as_mut()
            .into_iter()
            .for_each(|p| p.sigma_m = 0.0)),
        Err("position.sigma_m")
    );
    assert_eq!(
        refused(|s| s
            .position
            .as_mut()
            .into_iter()
            .for_each(|p| p.latitude_rad = 2.0)),
        Err("position.latitude_rad")
    );
    assert_eq!(
        refused(|s| s
            .height
            .as_mut()
            .into_iter()
            .for_each(|h| h.datum = VerticalDatum::Unknown)),
        Err("height.datum")
    );
    assert_eq!(
        refused(|s| s
            .heading
            .as_mut()
            .into_iter()
            .for_each(|h| h.heading_rad = 4.0)),
        Err("heading.heading_rad")
    );
    assert_eq!(
        refused(|s| s
            .speed
            .as_mut()
            .into_iter()
            .for_each(|v| v.speed_mps = -1.0)),
        Err("speed.speed_mps")
    );
    assert_eq!(
        refused(|s| s
            .speed
            .as_mut()
            .into_iter()
            .for_each(|v| v.sigma_mps = f64::NAN)),
        Err("speed.sigma_mps")
    );
}
