use crate::vertical::reported_height;

#[test]
fn reported_altitude_uses_terrain_sea_and_explicit_fallback() {
    let over_ground = reported_height(1_000.0, Some(400.0));
    let over_sea = reported_height(1_000.0, Some(0.0));
    let missing = reported_height(1_000.0, None);

    assert_eq!(over_ground.metres, 600.0);
    assert!(!over_ground.uses_reported_altitude_fallback);
    assert_eq!(over_sea.metres, 1_000.0);
    assert!(!over_sea.uses_reported_altitude_fallback);
    assert_eq!(missing.metres, 1_000.0);
    assert!(missing.uses_reported_altitude_fallback);
}
