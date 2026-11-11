//! A mission's identity is its content digest. The digest must not change
//! when a Navigate revision changes how a plan is held, because stored
//! records and mission documents carry it.

use navigate_contract::{ClockDomainId, GeodeticPosition};
use pilotage_mission::fixture::{self, GeoPointDegrees};
use pilotage_mission::{MissionConfig, MissionEngine, SurveyRequest, decode_snapshot};

const ANCHOR: GeoPointDegrees = GeoPointDegrees {
    lat_deg: 47.0,
    lon_deg: 8.0,
    alt_m: 500.0,
};

fn anchor() -> GeodeticPosition {
    GeodeticPosition::new(
        ANCHOR.lat_deg.to_radians(),
        ANCHOR.lon_deg.to_radians(),
        ANCHOR.alt_m,
    )
}

/// The digest of the demo route mission at the demo anchor.
const ROUTE_DIGEST: &str = "d2a5c07deb14b67a34b52390893fbbb16ff98ca7baeb86af131af168598c6dd4";
/// The digest of a 0.001° × 0.001° survey north-east of the anchor.
const SURVEY_DIGEST: &str = "5d2af8d87b05cb61788fe27e8c8ecfeeefeaded14a5bfd68795bc9044c9f7e5b";

#[test]
fn route_and_survey_missions_keep_their_content_digests() {
    let blob = fixture::demo_blob(ANCHOR).expect("demo blob encodes");
    let (snapshot, provenance) = decode_snapshot(&blob, true).expect("demo blob decodes");
    let config = MissionConfig::new(
        fixture::DEMO_ROUTE.to_owned(),
        anchor(),
        ClockDomainId::new(7),
    );
    let (_, route) = MissionEngine::new(&snapshot, provenance, config).expect("mission builds");

    let mut request = SurveyRequest::new(vec![
        [47.0, 8.0],
        [47.001, 8.0],
        [47.001, 8.001],
        [47.0, 8.001],
    ]);
    request.across_track_fov_rad = 1.5;
    let mut config = MissionConfig::new(String::new(), anchor(), ClockDomainId::new(7));
    config.cruise_height_m = 15.0;
    let (_, survey) = MissionEngine::from_survey(&request, config).expect("survey builds");

    let route_digest = route.mission_identity.content_digest.to_string();
    let survey_digest = survey.mission_identity.content_digest.to_string();
    assert_eq!(route_digest, ROUTE_DIGEST);
    assert_eq!(survey_digest, SURVEY_DIGEST);
}
